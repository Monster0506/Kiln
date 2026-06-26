#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <ctype.h>
#include <errno.h>

typedef struct { const char* ptr; int64_t len; } KilnStr;
typedef struct { int64_t* data; int64_t len; int64_t cap; } KilnVec;

static KilnStr* alloc_str_struct(const char* ptr, int64_t len) {
    KilnStr* s = (KilnStr*)malloc(sizeof(KilnStr));
    s->ptr = ptr; s->len = len;
    return s;
}

static int utf8_cp_len(unsigned char b) {
    if ((b & 0x80) == 0) return 1;
    if ((b & 0xE0) == 0xC0) return 2;
    if ((b & 0xF0) == 0xE0) return 3;
    if ((b & 0xF8) == 0xF0) return 4;
    return 1;
}

static int is_ws(unsigned char c) {
    return c == ' ' || c == '\t' || c == '\n' || c == '\r';
}

static int64_t make_option_none() {
    int32_t* box = (int32_t*)malloc(8);
    box[0] = 0; box[1] = 0;
    return (int64_t)box;
}
static int64_t make_option_some_i64(int64_t val) {
    int32_t* box = (int32_t*)malloc(16);
    box[0] = 1; box[1] = 0;
    *(int64_t*)(box + 2) = val;
    return (int64_t)box;
}
static int64_t make_option_some_f64(double val) {
    int32_t* box = (int32_t*)malloc(16);
    int64_t bits; memcpy(&bits, &val, 8);
    box[0] = 1; box[1] = 0;
    *(int64_t*)(box + 2) = bits;
    return (int64_t)box;
}

#define KILN_JMP_BUF_WORDS 8
#define MAX_EXC_DEPTH      64

static uint64_t* exc_ptrs[MAX_EXC_DEPTH];
static int       exc_depth = 0;
static int64_t   current_exc_val = 0;

#if defined(_WIN64) && (defined(__MINGW32__) || defined(__MINGW64__))
__asm__(
    ".globl __kiln_setjmp\n"
    "__kiln_setjmp:\n"
    "  movq   (%rsp), %rax\n"
    "  movq   %rax,  0(%rcx)\n"
    "  leaq   8(%rsp), %rax\n"
    "  movq   %rax,  8(%rcx)\n"
    "  movq   %rbx, 16(%rcx)\n"
    "  movq   %rbp, 24(%rcx)\n"
    "  movq   %r12, 32(%rcx)\n"
    "  movq   %r13, 40(%rcx)\n"
    "  movq   %r14, 48(%rcx)\n"
    "  movq   %r15, 56(%rcx)\n"
    "  xorl   %eax, %eax\n"
    "  retq\n"
    "\n"
    ".globl __kiln_longjmp\n"
    "__kiln_longjmp:\n"
    "  movq    0(%rcx), %r8\n"
    "  movq    8(%rcx), %rsp\n"
    "  movq   16(%rcx), %rbx\n"
    "  movq   24(%rcx), %rbp\n"
    "  movq   32(%rcx), %r12\n"
    "  movq   40(%rcx), %r13\n"
    "  movq   48(%rcx), %r14\n"
    "  movq   56(%rcx), %r15\n"
    "  movl   %edx, %eax\n"
    "  testl  %eax, %eax\n"
    "  jnz    1f\n"
    "  movl   $1, %eax\n"
    "1:\n"
    "  jmpq   *%r8\n"
);
extern "C" {
extern int32_t __kiln_setjmp(uint64_t* buf);
extern void    __kiln_longjmp(uint64_t* buf, int32_t val);
}
#elif defined(__x86_64__)
__asm__(
    ".globl __kiln_setjmp\n"
    "__kiln_setjmp:\n"
    "  movq   (%rsp), %rax\n"
    "  movq   %rax,  0(%rdi)\n"
    "  leaq   8(%rsp), %rax\n"
    "  movq   %rax,  8(%rdi)\n"
    "  movq   %rbx, 16(%rdi)\n"
    "  movq   %rbp, 24(%rdi)\n"
    "  movq   %r12, 32(%rdi)\n"
    "  movq   %r13, 40(%rdi)\n"
    "  movq   %r14, 48(%rdi)\n"
    "  movq   %r15, 56(%rdi)\n"
    "  xorl   %eax, %eax\n"
    "  retq\n"
    "\n"
    ".globl __kiln_longjmp\n"
    "__kiln_longjmp:\n"
    "  movq    0(%rdi), %r8\n"
    "  movq    8(%rdi), %rsp\n"
    "  movq   16(%rdi), %rbx\n"
    "  movq   24(%rdi), %rbp\n"
    "  movq   32(%rdi), %r12\n"
    "  movq   40(%rdi), %r13\n"
    "  movq   48(%rdi), %r14\n"
    "  movq   56(%rdi), %r15\n"
    "  movl   %esi, %eax\n"
    "  testl  %eax, %eax\n"
    "  jnz    1f\n"
    "  movl   $1, %eax\n"
    "1:\n"
    "  jmpq   *%r8\n"
);
extern "C" {
extern int32_t __kiln_setjmp(uint64_t* buf);
extern void    __kiln_longjmp(uint64_t* buf, int32_t val);
}
#else
#include <setjmp.h>
extern "C" {
int32_t __kiln_setjmp(uint64_t* buf) { (void)buf; return 0; }
void    __kiln_longjmp(uint64_t* buf, int32_t val) { (void)buf; (void)val; abort(); }
}
#endif

extern "C" {

void __kiln_print(int64_t str_val) {
    if (str_val == 0) return;
    KilnStr* s = (KilnStr*)str_val;
    fwrite(s->ptr, 1, (size_t)s->len, stdout);
}

void __kiln_println(int64_t str_val) {
    __kiln_print(str_val);
    putchar('\n');
}

int64_t __kiln_str_concat(int64_t a_val, int64_t b_val) {
    KilnStr* a = (KilnStr*)a_val;
    KilnStr* b = (KilnStr*)b_val;
    int64_t total = a->len + b->len;
    char* buf = (char*)malloc((size_t)(total + 1));
    memcpy(buf, a->ptr, (size_t)a->len);
    memcpy(buf + a->len, b->ptr, (size_t)b->len);
    buf[total] = 0;
    return (int64_t)alloc_str_struct(buf, total);
}

int64_t __kiln_int_to_str(int64_t n) {
    char buf[32];
    int len = snprintf(buf, sizeof(buf), "%lld", (long long)n);
    char* copy = (char*)malloc((size_t)(len + 1));
    memcpy(copy, buf, (size_t)(len + 1));
    return (int64_t)alloc_str_struct(copy, (int64_t)len);
}

int64_t __kiln_float_to_str(int64_t bits) {
    double f;
    memcpy(&f, &bits, 8);
    char buf[64];
    int len = snprintf(buf, sizeof(buf), "%g", f);
    char* copy = (char*)malloc((size_t)(len + 1));
    memcpy(copy, buf, (size_t)(len + 1));
    return (int64_t)alloc_str_struct(copy, (int64_t)len);
}

int64_t __kiln_bool_to_str(int64_t b) {
    const char* s = b ? "true" : "false";
    int64_t len = b ? 4 : 5;
    return (int64_t)alloc_str_struct(s, len);
}

void __kiln_rc_inc(int64_t ptr) {
    if (ptr == 0) return;
    int64_t* rc = (int64_t*)ptr;
    (*rc)++;
}

void __kiln_rc_dec(int64_t ptr) {
    if (ptr == 0) return;
    int64_t* rc = (int64_t*)ptr;
    if (--(*rc) <= 0) free((void*)ptr);
}

void __kiln_exc_push(uint64_t* buf) {
    if (exc_depth >= MAX_EXC_DEPTH) abort();
    exc_ptrs[exc_depth++] = buf;
}

void __kiln_exc_pop() {
    if (exc_depth > 0) exc_depth--;
}

void __kiln_raise(int64_t exc_ptr) {
    current_exc_val = exc_ptr;
    if (exc_depth > 0) {
        uint64_t* buf = exc_ptrs[exc_depth - 1];
        __kiln_longjmp(buf, 1);
    }
    fprintf(stderr, "Unhandled Kiln exception\n");
    abort();
}

int64_t __kiln_current_exc() { return current_exc_val; }

int64_t Vec_new() {
    KilnVec* v = (KilnVec*)malloc(sizeof(KilnVec));
    v->data = NULL; v->len = 0; v->cap = 0;
    return (int64_t)v;
}

void Vec_add(int64_t vec_ptr, int64_t item) {
    KilnVec* v = (KilnVec*)vec_ptr;
    if (v->len >= v->cap) {
        int64_t new_cap = v->cap == 0 ? 4 : v->cap * 2;
        v->data = (int64_t*)realloc(v->data, (size_t)new_cap * sizeof(int64_t));
        v->cap = new_cap;
    }
    v->data[v->len++] = item;
}

int64_t Vec_len(int64_t vec_ptr) { return ((KilnVec*)vec_ptr)->len; }

int64_t Vec_get(int64_t vec_ptr, int64_t index) {
    KilnVec* v = (KilnVec*)vec_ptr;
    if (index < 0) index += v->len;
    return v->data[index];
}

void Vec_set(int64_t vec_ptr, int64_t index, int64_t item) {
    KilnVec* v = (KilnVec*)vec_ptr;
    if (index < 0) index += v->len;
    v->data[index] = item;
}

void Vec_clear(int64_t vec_ptr) { ((KilnVec*)vec_ptr)->len = 0; }

int64_t Vec_remove(int64_t vec_ptr, int64_t index) {
    KilnVec* v = (KilnVec*)vec_ptr;
    int64_t val = v->data[index];
    for (int64_t i = index; i < v->len - 1; i++) v->data[i] = v->data[i + 1];
    v->len--;
    return val;
}

int64_t __kiln_to_str_dispatch(int64_t val) {
    if (val == 0) return (int64_t)alloc_str_struct("null", 4);
    KilnStr* maybe = (KilnStr*)val;
    if (maybe->ptr != NULL && maybe->len >= 0 && maybe->len < (1 << 26)) return val;
    return __kiln_int_to_str(val);
}

int64_t __kiln_spawn(int64_t fn_ptr, int64_t env_ptr) {
    typedef int64_t (*FnPtr)(int64_t);
    return ((FnPtr)fn_ptr)(env_ptr);
}

int64_t __kiln_str_byte_len(int64_t str_val) {
    if (str_val == 0) return 0;
    return ((KilnStr*)str_val)->len;
}

int64_t __kiln_str_codepoint_len(int64_t str_val) {
    if (str_val == 0) return 0;
    KilnStr* s = (KilnStr*)str_val;
    const unsigned char* p = (const unsigned char*)s->ptr;
    int64_t count = 0, i = 0;
    while (i < s->len) { i += utf8_cp_len(p[i]); count++; }
    return count;
}

int64_t __kiln_str_char_at(int64_t str_val, int64_t idx) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    const unsigned char* p = (const unsigned char*)s->ptr;
    int64_t count = 0, i = 0;
    while (i < s->len) {
        int cp_len = utf8_cp_len(p[i]);
        if (count == idx) {
            char* buf = (char*)malloc((size_t)(cp_len + 1));
            memcpy(buf, p + i, (size_t)cp_len);
            buf[cp_len] = '\0';
            return (int64_t)alloc_str_struct(buf, (int64_t)cp_len);
        }
        i += cp_len; count++;
    }
    return (int64_t)alloc_str_struct("", 0);
}

int64_t __kiln_str_byte_at(int64_t str_val, int64_t idx) {
    if (str_val == 0) return 0;
    KilnStr* s = (KilnStr*)str_val;
    if (idx < 0 || idx >= s->len) return 0;
    return (int64_t)(unsigned char)s->ptr[idx];
}

int64_t __kiln_str_slice(int64_t str_val, int64_t start_cp, int64_t end_cp) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    const unsigned char* p = (const unsigned char*)s->ptr;
    int64_t byte_start = -1, byte_end = -1, count = 0, i = 0;
    while (i <= s->len) {
        if (count == start_cp) byte_start = i;
        if (count == end_cp) { byte_end = i; break; }
        if (i == s->len) break;
        i += utf8_cp_len(p[i]); count++;
    }
    if (byte_start < 0) byte_start = s->len;
    if (byte_end < 0)   byte_end   = s->len;
    int64_t len = byte_end - byte_start;
    if (len <= 0) return (int64_t)alloc_str_struct("", 0);
    char* buf = (char*)malloc((size_t)(len + 1));
    memcpy(buf, p + byte_start, (size_t)len);
    buf[len] = '\0';
    return (int64_t)alloc_str_struct(buf, len);
}

int64_t __kiln_str_starts_with(int64_t str_val, int64_t pfx_val) {
    if (str_val == 0 || pfx_val == 0) return 0;
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* p = (KilnStr*)pfx_val;
    if (p->len == 0) return 1;
    if (p->len > s->len) return 0;
    return memcmp(s->ptr, p->ptr, (size_t)p->len) == 0 ? 1 : 0;
}

int64_t __kiln_str_ends_with(int64_t str_val, int64_t sfx_val) {
    if (str_val == 0 || sfx_val == 0) return 0;
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* sfx = (KilnStr*)sfx_val;
    if (sfx->len == 0) return 1;
    if (sfx->len > s->len) return 0;
    return memcmp(s->ptr + s->len - sfx->len, sfx->ptr, (size_t)sfx->len) == 0 ? 1 : 0;
}

int64_t __kiln_str_find_from(int64_t str_val, int64_t needle_val, int64_t start_cp) {
    if (str_val == 0 || needle_val == 0) return -1;
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* n = (KilnStr*)needle_val;
    if (n->len == 0) return start_cp;
    const unsigned char* sp = (const unsigned char*)s->ptr;
    int64_t count = 0, i = 0;
    while (i < s->len && count < start_cp) { i += utf8_cp_len(sp[i]); count++; }
    while (i + n->len <= s->len) {
        if (memcmp(sp + i, n->ptr, (size_t)n->len) == 0) return count;
        i += utf8_cp_len(sp[i]); count++;
    }
    return -1;
}

int64_t __kiln_str_find(int64_t str_val, int64_t needle_val) {
    return __kiln_str_find_from(str_val, needle_val, 0);
}

int64_t __kiln_str_contains(int64_t str_val, int64_t needle_val) {
    return __kiln_str_find(str_val, needle_val) >= 0 ? 1 : 0;
}

int64_t __kiln_str_to_upper(int64_t str_val) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    char* buf = (char*)malloc((size_t)(s->len + 1));
    for (int64_t i = 0; i < s->len; i++) buf[i] = (char)toupper((unsigned char)s->ptr[i]);
    buf[s->len] = '\0';
    return (int64_t)alloc_str_struct(buf, s->len);
}

int64_t __kiln_str_to_lower(int64_t str_val) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    char* buf = (char*)malloc((size_t)(s->len + 1));
    for (int64_t i = 0; i < s->len; i++) buf[i] = (char)tolower((unsigned char)s->ptr[i]);
    buf[s->len] = '\0';
    return (int64_t)alloc_str_struct(buf, s->len);
}

int64_t __kiln_str_reverse(int64_t str_val) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    if (s->len == 0) return (int64_t)alloc_str_struct("", 0);
    const unsigned char* p = (const unsigned char*)s->ptr;
    int64_t* starts = (int64_t*)malloc((size_t)(s->len + 1) * sizeof(int64_t));
    int*     lens   = (int*)malloc((size_t)(s->len + 1) * sizeof(int));
    int64_t ncp = 0, i = 0;
    while (i < s->len) {
        int cplen = utf8_cp_len(p[i]);
        starts[ncp] = i; lens[ncp] = cplen;
        i += cplen; ncp++;
    }
    char* buf = (char*)malloc((size_t)(s->len + 1));
    int64_t out = 0;
    for (int64_t k = ncp - 1; k >= 0; k--) {
        memcpy(buf + out, p + starts[k], (size_t)lens[k]);
        out += lens[k];
    }
    buf[s->len] = '\0';
    free(starts); free(lens);
    return (int64_t)alloc_str_struct(buf, s->len);
}

int64_t __kiln_str_trim_start(int64_t str_val) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    const unsigned char* p = (const unsigned char*)s->ptr;
    int64_t start = 0;
    while (start < s->len && is_ws(p[start])) start++;
    int64_t len = s->len - start;
    char* buf = (char*)malloc((size_t)(len + 1));
    memcpy(buf, p + start, (size_t)len);
    buf[len] = '\0';
    return (int64_t)alloc_str_struct(buf, len);
}

int64_t __kiln_str_trim_end(int64_t str_val) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    const unsigned char* p = (const unsigned char*)s->ptr;
    int64_t end = s->len;
    while (end > 0 && is_ws(p[end - 1])) end--;
    char* buf = (char*)malloc((size_t)(end + 1));
    memcpy(buf, p, (size_t)end);
    buf[end] = '\0';
    return (int64_t)alloc_str_struct(buf, end);
}

int64_t __kiln_str_trim(int64_t str_val) {
    return __kiln_str_trim_end(__kiln_str_trim_start(str_val));
}

int64_t __kiln_str_replace(int64_t str_val, int64_t from_val, int64_t to_val) {
    if (str_val == 0 || from_val == 0) return str_val;
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* f = (KilnStr*)from_val;
    KilnStr* t = to_val ? (KilnStr*)to_val : NULL;
    const char* tp = t ? t->ptr : "";
    int64_t tlen = t ? t->len : 0;
    if (f->len == 0) return str_val;
    const char* pos = NULL;
    for (int64_t i = 0; i <= s->len - f->len; i++) {
        if (memcmp(s->ptr + i, f->ptr, (size_t)f->len) == 0) { pos = s->ptr + i; break; }
    }
    if (!pos) return str_val;
    int64_t before = pos - s->ptr;
    int64_t after_start = before + f->len;
    int64_t after = s->len - after_start;
    int64_t new_len = before + tlen + after;
    char* buf = (char*)malloc((size_t)(new_len + 1));
    memcpy(buf, s->ptr, (size_t)before);
    memcpy(buf + before, tp, (size_t)tlen);
    memcpy(buf + before + tlen, s->ptr + after_start, (size_t)after);
    buf[new_len] = '\0';
    return (int64_t)alloc_str_struct(buf, new_len);
}

int64_t __kiln_str_repeat(int64_t str_val, int64_t n) {
    if (str_val == 0 || n <= 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    int64_t new_len = s->len * n;
    char* buf = (char*)malloc((size_t)(new_len + 1));
    for (int64_t i = 0; i < n; i++) memcpy(buf + i * s->len, s->ptr, (size_t)s->len);
    buf[new_len] = '\0';
    return (int64_t)alloc_str_struct(buf, new_len);
}

int64_t __kiln_str_split(int64_t str_val, int64_t sep_val) {
    int64_t vec = Vec_new();
    if (str_val == 0) return vec;
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* sep = (KilnStr*)sep_val;
    if (!sep || sep->len == 0) { Vec_add(vec, str_val); return vec; }
    int64_t start = 0;
    for (int64_t i = 0; i <= s->len - sep->len; ) {
        if (memcmp(s->ptr + i, sep->ptr, (size_t)sep->len) == 0) {
            int64_t part_len = i - start;
            char* buf = (char*)malloc((size_t)(part_len + 1));
            memcpy(buf, s->ptr + start, (size_t)part_len);
            buf[part_len] = '\0';
            Vec_add(vec, (int64_t)alloc_str_struct(buf, part_len));
            i += sep->len; start = i;
        } else { i++; }
    }
    int64_t last_len = s->len - start;
    char* buf = (char*)malloc((size_t)(last_len + 1));
    memcpy(buf, s->ptr + start, (size_t)last_len);
    buf[last_len] = '\0';
    Vec_add(vec, (int64_t)alloc_str_struct(buf, last_len));
    return vec;
}

int64_t __kiln_str_split_whitespace(int64_t str_val) {
    int64_t vec = Vec_new();
    if (str_val == 0) return vec;
    KilnStr* s = (KilnStr*)str_val;
    const unsigned char* p = (const unsigned char*)s->ptr;
    int64_t i = 0;
    while (i < s->len) {
        while (i < s->len && is_ws(p[i])) i++;
        if (i >= s->len) break;
        int64_t start = i;
        while (i < s->len && !is_ws(p[i])) i++;
        int64_t part_len = i - start;
        char* buf = (char*)malloc((size_t)(part_len + 1));
        memcpy(buf, p + start, (size_t)part_len);
        buf[part_len] = '\0';
        Vec_add(vec, (int64_t)alloc_str_struct(buf, part_len));
    }
    return vec;
}

int64_t __kiln_str_chars(int64_t str_val) {
    int64_t vec = Vec_new();
    if (str_val == 0) return vec;
    KilnStr* s = (KilnStr*)str_val;
    const unsigned char* p = (const unsigned char*)s->ptr;
    int64_t i = 0;
    while (i < s->len) {
        int cplen = utf8_cp_len(p[i]);
        char* buf = (char*)malloc((size_t)(cplen + 1));
        memcpy(buf, p + i, (size_t)cplen);
        buf[cplen] = '\0';
        Vec_add(vec, (int64_t)alloc_str_struct(buf, (int64_t)cplen));
        i += cplen;
    }
    return vec;
}

int64_t __kiln_str_bytes_vec(int64_t str_val) {
    int64_t vec = Vec_new();
    if (str_val == 0) return vec;
    KilnStr* s = (KilnStr*)str_val;
    for (int64_t i = 0; i < s->len; i++) Vec_add(vec, (int64_t)(unsigned char)s->ptr[i]);
    return vec;
}

int64_t __kiln_str_pad_start(int64_t str_val, int64_t width, int64_t pad_val) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* p = pad_val ? (KilnStr*)pad_val : NULL;
    int64_t pad_len = p ? p->len : 1;
    int64_t cp_len = __kiln_str_codepoint_len(str_val);
    if (cp_len >= width) return str_val;
    int64_t pad_cps = width - cp_len;
    int64_t total = pad_cps * pad_len + s->len;
    char* buf = (char*)malloc((size_t)(total + 1));
    const char* pc = p ? p->ptr : " ";
    for (int64_t i = 0; i < pad_cps; i++) memcpy(buf + i * pad_len, pc, (size_t)pad_len);
    memcpy(buf + pad_cps * pad_len, s->ptr, (size_t)s->len);
    buf[total] = '\0';
    return (int64_t)alloc_str_struct(buf, total);
}

int64_t __kiln_str_pad_end(int64_t str_val, int64_t width, int64_t pad_val) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* p = pad_val ? (KilnStr*)pad_val : NULL;
    int64_t pad_len = p ? p->len : 1;
    int64_t cp_len = __kiln_str_codepoint_len(str_val);
    if (cp_len >= width) return str_val;
    int64_t pad_cps = width - cp_len;
    int64_t total = s->len + pad_cps * pad_len;
    char* buf = (char*)malloc((size_t)(total + 1));
    memcpy(buf, s->ptr, (size_t)s->len);
    const char* pc = p ? p->ptr : " ";
    for (int64_t i = 0; i < pad_cps; i++)
        memcpy(buf + s->len + i * pad_len, pc, (size_t)pad_len);
    buf[total] = '\0';
    return (int64_t)alloc_str_struct(buf, total);
}

int64_t __kiln_str_remove_prefix(int64_t str_val, int64_t pfx_val) {
    if (!str_val || !pfx_val) return str_val ? str_val : (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* p = (KilnStr*)pfx_val;
    if (p->len == 0 || p->len > s->len || memcmp(s->ptr, p->ptr, (size_t)p->len) != 0)
        return str_val;
    int64_t new_len = s->len - p->len;
    char* buf = (char*)malloc((size_t)(new_len + 1));
    memcpy(buf, s->ptr + p->len, (size_t)new_len);
    buf[new_len] = '\0';
    return (int64_t)alloc_str_struct(buf, new_len);
}

int64_t __kiln_str_remove_suffix(int64_t str_val, int64_t sfx_val) {
    if (!str_val || !sfx_val) return str_val ? str_val : (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* sfx = (KilnStr*)sfx_val;
    if (sfx->len == 0 || sfx->len > s->len ||
        memcmp(s->ptr + s->len - sfx->len, sfx->ptr, (size_t)sfx->len) != 0)
        return str_val;
    int64_t new_len = s->len - sfx->len;
    char* buf = (char*)malloc((size_t)(new_len + 1));
    memcpy(buf, s->ptr, (size_t)new_len);
    buf[new_len] = '\0';
    return (int64_t)alloc_str_struct(buf, new_len);
}

int64_t __kiln_str_parse_int(int64_t str_val) {
    if (!str_val) return make_option_none();
    KilnStr* s = (KilnStr*)str_val;
    char* buf = (char*)malloc((size_t)(s->len + 1));
    memcpy(buf, s->ptr, (size_t)s->len);
    buf[s->len] = '\0';
    char* end = buf;
    errno = 0;
    long long v = strtoll(buf, &end, 10);
    int ok = (errno == 0 && end != buf && *end == '\0');
    free(buf);
    return ok ? make_option_some_i64((int64_t)v) : make_option_none();
}

int64_t __kiln_str_parse_float(int64_t str_val) {
    if (!str_val) return make_option_none();
    KilnStr* s = (KilnStr*)str_val;
    char* buf = (char*)malloc((size_t)(s->len + 1));
    memcpy(buf, s->ptr, (size_t)s->len);
    buf[s->len] = '\0';
    char* end = buf;
    errno = 0;
    double v = strtod(buf, &end);
    int ok = (errno == 0 && end != buf && *end == '\0');
    free(buf);
    return ok ? make_option_some_f64(v) : make_option_none();
}

} // extern "C"
