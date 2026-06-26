#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

typedef struct { const char* ptr; int64_t len; } KilnStr;

static KilnStr* alloc_str_struct(const char* ptr, int64_t len) {
    KilnStr* s = (KilnStr*)malloc(sizeof(KilnStr));
    s->ptr = ptr;
    s->len = len;
    return s;
}

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

/* ---- Exception runtime -------------------------------------------------------
 *
 * Design: the jmp_buf lives in the *caller's* (Cranelift-generated) stack
 * frame as a 64-byte StackSlot, not in a C helper function.  Cranelift calls
 * __kiln_exc_push(buf_ptr) to register the frame, then calls __kiln_setjmp
 * directly.  __kiln_raise calls __kiln_longjmp which restores registers and
 * jumps back to the instruction after the __kiln_setjmp call inside the
 * Cranelift function (whose frame is still live on the stack).
 *
 * The custom __kiln_setjmp / __kiln_longjmp bypass the C-library
 * setjmp/longjmp entirely, which on Windows 64-bit would call RtlUnwindEx and
 * crash when Cranelift-generated frames (lacking .pdata tables) are on the
 * call stack.
 *
 * jmp_buf layout (8 x uint64_t = 64 bytes):
 *   [0]  rip  -- return address from __kiln_setjmp (= next insn in caller)
 *   [1]  rsp  -- caller's rsp after the call instruction ret-pops rip
 *   [2]  rbx
 *   [3]  rbp
 *   [4]  r12
 *   [5]  r13
 *   [6]  r14
 *   [7]  r15
 * --------------------------------------------------------------------------- */

#define KILN_JMP_BUF_WORDS 8
#define MAX_EXC_DEPTH      64

static uint64_t* exc_ptrs[MAX_EXC_DEPTH];
static int       exc_depth = 0;
static int64_t   current_exc_val = 0;

/* Custom setjmp/longjmp in global assembly.
 * We define them here and provide extern declarations below.
 * On Windows x64 the first integer argument is in RCX, second in RDX.
 * On System V x64 (Linux/macOS) the first is in RDI, second in RSI.      */

#if defined(_WIN64) && (defined(__MINGW32__) || defined(__MINGW64__))

__asm__(
    ".globl __kiln_setjmp\n"
    "__kiln_setjmp:\n"
    "  movq   (%rsp), %rax\n"      /* return address (= next insn in caller) */
    "  movq   %rax,  0(%rcx)\n"    /* buf[0] = rip */
    "  leaq   8(%rsp), %rax\n"     /* caller rsp after the call's ret pops rip */
    "  movq   %rax,  8(%rcx)\n"    /* buf[1] = rsp */
    "  movq   %rbx, 16(%rcx)\n"
    "  movq   %rbp, 24(%rcx)\n"
    "  movq   %r12, 32(%rcx)\n"
    "  movq   %r13, 40(%rcx)\n"
    "  movq   %r14, 48(%rcx)\n"
    "  movq   %r15, 56(%rcx)\n"
    "  xorl   %eax, %eax\n"        /* return 0 */
    "  retq\n"
    "\n"
    ".globl __kiln_longjmp\n"
    "__kiln_longjmp:\n"
    "  movq    0(%rcx), %r8\n"     /* saved rip */
    "  movq    8(%rcx), %rsp\n"    /* restore caller rsp */
    "  movq   16(%rcx), %rbx\n"
    "  movq   24(%rcx), %rbp\n"
    "  movq   32(%rcx), %r12\n"
    "  movq   40(%rcx), %r13\n"
    "  movq   48(%rcx), %r14\n"
    "  movq   56(%rcx), %r15\n"
    "  movl   %edx, %eax\n"        /* return val */
    "  testl  %eax, %eax\n"
    "  jnz    1f\n"
    "  movl   $1, %eax\n"          /* longjmp(buf,0) acts like longjmp(buf,1) */
    "1:\n"
    "  jmpq   *%r8\n"              /* jump to saved rip in caller */
);

extern int32_t __kiln_setjmp(uint64_t* buf);
extern void    __kiln_longjmp(uint64_t* buf, int32_t val);

#elif defined(__x86_64__)

/* System V AMD64 ABI: first arg in RDI, second in RSI. */
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

extern int32_t __kiln_setjmp(uint64_t* buf);
extern void    __kiln_longjmp(uint64_t* buf, int32_t val);

#else
/* Fallback for other architectures: use C-library setjmp/longjmp.
   May crash if the C library's longjmp calls OS unwinding APIs that require
   unwind tables in the Cranelift-generated code. */
#include <setjmp.h>
typedef jmp_buf kiln_fallback_buf;
static kiln_fallback_buf fallback_exc_stack[MAX_EXC_DEPTH];
/* These are stubs; the actual calling convention differs on other arches. */
int32_t __kiln_setjmp(uint64_t* buf) {
    (void)buf; return 0;
}
void __kiln_longjmp(uint64_t* buf, int32_t val) {
    (void)buf; (void)val; abort();
}
#endif

/* Register a jmp_buf (allocated by the Cranelift caller) as the active frame. */
void __kiln_exc_push(uint64_t* buf) {
    if (exc_depth >= MAX_EXC_DEPTH) abort();
    exc_ptrs[exc_depth++] = buf;
}

/* Unregister the innermost exception frame. */
void __kiln_exc_pop(void) {
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

int64_t __kiln_current_exc(void) { return current_exc_val; }

/* ---- Vec ------------------------------------------------------------------ */

typedef struct {
    int64_t* data;
    int64_t len;
    int64_t cap;
} KilnVec;

int64_t Vec_new(void) {
    KilnVec* v = (KilnVec*)malloc(sizeof(KilnVec));
    v->data = NULL;
    v->len = 0;
    v->cap = 0;
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

int64_t Vec_len(int64_t vec_ptr) {
    return ((KilnVec*)vec_ptr)->len;
}

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

void Vec_clear(int64_t vec_ptr) {
    ((KilnVec*)vec_ptr)->len = 0;
}

int64_t Vec_remove(int64_t vec_ptr, int64_t index) {
    KilnVec* v = (KilnVec*)vec_ptr;
    int64_t val = v->data[index];
    for (int64_t i = index; i < v->len - 1; i++) {
        v->data[i] = v->data[i + 1];
    }
    v->len--;
    return val;
}

/* Generic to_str dispatcher for values of unknown type (used in generic hooks).
   Without runtime type tags a perfect implementation is not possible; this
   handles the common primitive cases and falls back to int formatting.
   Vec[int] elements will render correctly; Vec[str] elements will render as
   addresses until proper type tagging is added. */
int64_t __kiln_to_str_dispatch(int64_t val) {
    if (val == 0) {
        return (int64_t)alloc_str_struct("null", 4);
    }
    /* Heuristic: try to detect a KilnStr pointer by checking that the
       pointed-to region looks plausible (ptr != NULL, 0 <= len < 64 MB).
       This works reliably when the Vec element type is str; for other
       heap-allocated types the output will be a decimal address, which is
       still safe (no crash). */
    KilnStr* maybe = (KilnStr*)val;
    if (maybe->ptr != NULL && maybe->len >= 0 && maybe->len < (1 << 26)) {
        return val;
    }
    return __kiln_int_to_str(val);
}

int64_t __kiln_spawn(int64_t fn_ptr, int64_t env_ptr) {
    typedef int64_t (*FnPtr)(int64_t);
    return ((FnPtr)fn_ptr)(env_ptr);
}

/* ---- String methods -------------------------------------------------------- */

static int utf8_cp_len(unsigned char b) {
    if ((b & 0x80) == 0) return 1;
    if ((b & 0xE0) == 0xC0) return 2;
    if ((b & 0xF0) == 0xE0) return 3;
    if ((b & 0xF8) == 0xF0) return 4;
    return 1;
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
        i += cp_len;
        count++;
    }
    return (int64_t)alloc_str_struct("", 0);
}

int64_t __kiln_str_byte_at(int64_t str_val, int64_t idx) {
    if (str_val == 0) return 0;
    KilnStr* s = (KilnStr*)str_val;
    if (idx < 0 || idx >= s->len) return 0;
    return (int64_t)(unsigned char)s->ptr[idx];
}

/* slice(start_cp, end_cp) -> str: byte slice by codepoint indices */
int64_t __kiln_str_slice(int64_t str_val, int64_t start_cp, int64_t end_cp) {
    if (str_val == 0) return (int64_t)alloc_str_struct("", 0);
    KilnStr* s = (KilnStr*)str_val;
    const unsigned char* p = (const unsigned char*)s->ptr;
    int64_t byte_start = -1, byte_end = -1, count = 0, i = 0;
    while (i <= s->len) {
        if (count == start_cp) byte_start = i;
        if (count == end_cp)   { byte_end = i; break; }
        if (i == s->len) break;
        i += utf8_cp_len(p[i]);
        count++;
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

/* starts_with(prefix) -> bool (1/0) */
int64_t __kiln_str_starts_with(int64_t str_val, int64_t pfx_val) {
    if (str_val == 0 || pfx_val == 0) return 0;
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* p = (KilnStr*)pfx_val;
    if (p->len == 0) return 1;
    if (p->len > s->len) return 0;
    return memcmp(s->ptr, p->ptr, (size_t)p->len) == 0 ? 1 : 0;
}

/* ends_with(suffix) -> bool (1/0) */
int64_t __kiln_str_ends_with(int64_t str_val, int64_t sfx_val) {
    if (str_val == 0 || sfx_val == 0) return 0;
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* sfx = (KilnStr*)sfx_val;
    if (sfx->len == 0) return 1;
    if (sfx->len > s->len) return 0;
    return memcmp(s->ptr + s->len - sfx->len, sfx->ptr, (size_t)sfx->len) == 0 ? 1 : 0;
}

/* find_from(needle, start_byte) -> codepoint index or -1 */
int64_t __kiln_str_find_from(int64_t str_val, int64_t needle_val, int64_t start_cp) {
    if (str_val == 0 || needle_val == 0) return -1;
    KilnStr* s = (KilnStr*)str_val;
    KilnStr* n = (KilnStr*)needle_val;
    if (n->len == 0) return start_cp;
    const unsigned char* sp = (const unsigned char*)s->ptr;
    int64_t count = 0, i = 0;
    /* advance to start_cp */
    while (i < s->len && count < start_cp) {
        i += utf8_cp_len(sp[i]);
        count++;
    }
    while (i + n->len <= s->len) {
        if (memcmp(sp + i, n->ptr, (size_t)n->len) == 0) return count;
        i += utf8_cp_len(sp[i]);
        count++;
    }
    return -1;
}

/* find(needle) -> codepoint index or -1 */
int64_t __kiln_str_find(int64_t str_val, int64_t needle_val) {
    return __kiln_str_find_from(str_val, needle_val, 0);
}

/* contains(needle) -> bool */
int64_t __kiln_str_contains(int64_t str_val, int64_t needle_val) {
    return __kiln_str_find(str_val, needle_val) >= 0 ? 1 : 0;
}
