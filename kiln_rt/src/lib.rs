#![allow(non_snake_case)]

// Kiln HTTP runtime
// Exports Http_get/post/put/patch/delete/head/options and Response_* as
// C-ABI functions matching builtins.kn @builtin struct declarations.

// Kiln str: {ptr: *const u8, len: i64} (16 bytes on 64-bit)
#[repr(C)]
struct KilnStr {
    ptr: *const u8,
    len: i64,
}

// Heap block for Result/Option Some: {i32 disc, i32 pad, i64 payload}
#[repr(C)]
struct KilnBlock {
    disc: i32,
    _pad: i32,
    payload: i64,
}

// Our HTTP response -- held on the heap, pointer passed as i64.
struct HttpResp {
    status: i64,
    body: String,
    // lowercase header name -> value
    headers: Vec<(String, String)>,
}

unsafe fn read_kiln_str<'a>(raw: i64) -> &'a str {
    // Use unaligned reads: Kiln's Cranelift codegen places KilnStr data objects
    // in the data section without guaranteeing 8-byte alignment.
    let base = raw as *const u8;
    let str_ptr: *const u8 = std::ptr::read_unaligned(base as *const *const u8);
    let len: i64 = std::ptr::read_unaligned(base.add(8) as *const i64);
    std::str::from_utf8_unchecked(std::slice::from_raw_parts(str_ptr, len as usize))
}

fn alloc_kiln_str(s: &str) -> i64 {
    let bytes = s.as_bytes().to_vec().into_boxed_slice();
    let ptr = bytes.as_ptr();
    let len = bytes.len() as i64;
    std::mem::forget(bytes);
    Box::into_raw(Box::new(KilnStr { ptr, len })) as i64
}

fn make_result_ok(val: i64) -> i64 {
    Box::into_raw(Box::new(KilnBlock { disc: 0, _pad: 0, payload: val })) as i64
}

fn make_result_err(payload: i64) -> i64 {
    Box::into_raw(Box::new(KilnBlock { disc: 1, _pad: 0, payload })) as i64
}

// Option::None is the raw integer 1 (not a pointer).
const KILN_NONE: i64 = 1;

fn option_some(val: i64) -> i64 {
    // disc=0 matches kiln_option_some in kiln_rt.cpp
    Box::into_raw(Box::new(KilnBlock { disc: 0, _pad: 0, payload: val })) as i64
}

// HttpError struct layout in Kiln: [msg: i64, status: i64] (two i64 fields).
fn alloc_http_error(msg: &str, status: i64) -> i64 {
    let msg_ptr = alloc_kiln_str(msg);
    let fields = Box::new([msg_ptr, status]);
    Box::into_raw(fields) as *mut i64 as i64
}

fn finish_resp(resp: reqwest::blocking::Response, label: &str) -> Result<HttpResp, String> {
    let status = resp.status().as_u16() as i64;
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_ascii_lowercase(),
                v.to_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    let body = resp
        .text()
        .map_err(|e| format!("Http.{label}: read body: {e}"))?;
    Ok(HttpResp { status, body, headers })
}

fn do_get(url: &str) -> Result<HttpResp, String> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("Http.get: {e}"))?;
    let resp = client.get(url).send().map_err(|e| format!("Http.get: {e}"))?;
    finish_resp(resp, "get")
}

fn do_post(url: &str, body: &str, ct: &str) -> Result<HttpResp, String> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("Http.post: {e}"))?;
    let resp = client
        .post(url)
        .header("Content-Type", ct)
        .body(body.to_owned())
        .send()
        .map_err(|e| format!("Http.post: {e}"))?;
    finish_resp(resp, "post")
}

fn do_put(url: &str, body: &str, ct: &str) -> Result<HttpResp, String> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("Http.put: {e}"))?;
    let resp = client
        .put(url)
        .header("Content-Type", ct)
        .body(body.to_owned())
        .send()
        .map_err(|e| format!("Http.put: {e}"))?;
    finish_resp(resp, "put")
}

fn do_patch(url: &str, body: &str, ct: &str) -> Result<HttpResp, String> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("Http.patch: {e}"))?;
    let resp = client
        .patch(url)
        .header("Content-Type", ct)
        .body(body.to_owned())
        .send()
        .map_err(|e| format!("Http.patch: {e}"))?;
    finish_resp(resp, "patch")
}

fn do_delete(url: &str) -> Result<HttpResp, String> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("Http.delete: {e}"))?;
    let resp = client.delete(url).send().map_err(|e| format!("Http.delete: {e}"))?;
    finish_resp(resp, "delete")
}

fn do_head(url: &str) -> Result<HttpResp, String> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("Http.head: {e}"))?;
    let resp = client.head(url).send().map_err(|e| format!("Http.head: {e}"))?;
    finish_resp(resp, "head")
}

fn do_options(url: &str) -> Result<HttpResp, String> {
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| format!("Http.options: {e}"))?;
    let resp = client
        .request(reqwest::Method::OPTIONS, url)
        .send()
        .map_err(|e| format!("Http.options: {e}"))?;
    finish_resp(resp, "options")
}

fn wrap(r: Result<HttpResp, String>) -> i64 {
    match r {
        Ok(resp) => make_result_ok(Box::into_raw(Box::new(resp)) as i64),
        Err(e) => make_result_err(alloc_http_error(&e, 0)),
    }
}

#[no_mangle]
pub unsafe extern "C" fn Http_get(url_raw: i64) -> i64 {
    wrap(do_get(read_kiln_str(url_raw)))
}

#[no_mangle]
pub unsafe extern "C" fn Http_post(url_raw: i64, body_raw: i64, ct_raw: i64) -> i64 {
    wrap(do_post(read_kiln_str(url_raw), read_kiln_str(body_raw), read_kiln_str(ct_raw)))
}

#[no_mangle]
pub unsafe extern "C" fn Http_put(url_raw: i64, body_raw: i64, ct_raw: i64) -> i64 {
    wrap(do_put(read_kiln_str(url_raw), read_kiln_str(body_raw), read_kiln_str(ct_raw)))
}

#[no_mangle]
pub unsafe extern "C" fn Http_patch(url_raw: i64, body_raw: i64, ct_raw: i64) -> i64 {
    wrap(do_patch(read_kiln_str(url_raw), read_kiln_str(body_raw), read_kiln_str(ct_raw)))
}

#[no_mangle]
pub unsafe extern "C" fn Http_delete(url_raw: i64) -> i64 {
    wrap(do_delete(read_kiln_str(url_raw)))
}

#[no_mangle]
pub unsafe extern "C" fn Http_head(url_raw: i64) -> i64 {
    wrap(do_head(read_kiln_str(url_raw)))
}

#[no_mangle]
pub unsafe extern "C" fn Http_options(url_raw: i64) -> i64 {
    wrap(do_options(read_kiln_str(url_raw)))
}

#[no_mangle]
pub unsafe extern "C" fn Response_status(resp_raw: i64) -> i64 {
    let resp = &*(resp_raw as *const HttpResp);
    resp.status
}

#[no_mangle]
pub unsafe extern "C" fn Response_body(resp_raw: i64) -> i64 {
    let resp = &*(resp_raw as *const HttpResp);
    alloc_kiln_str(&resp.body)
}

#[no_mangle]
pub unsafe extern "C" fn Response_ok(resp_raw: i64) -> i64 {
    let resp = &*(resp_raw as *const HttpResp);
    (resp.status >= 200 && resp.status < 300) as i64
}

#[no_mangle]
pub unsafe extern "C" fn Response_header(resp_raw: i64, name_raw: i64) -> i64 {
    let resp = &*(resp_raw as *const HttpResp);
    let name = read_kiln_str(name_raw).to_ascii_lowercase();
    match resp.headers.iter().find(|(k, _)| k == &name) {
        Some((_, v)) => option_some(alloc_kiln_str(v)),
        None => KILN_NONE,
    }
}
