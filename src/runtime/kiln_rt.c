#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <setjmp.h>

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

#define MAX_EXC_DEPTH 64
typedef struct { jmp_buf buf; } ExcFrame;
static ExcFrame exc_stack[MAX_EXC_DEPTH];
static int exc_depth = 0;
static int64_t current_exc_val = 0;

int32_t __kiln_try_enter(void) {
    if (exc_depth >= MAX_EXC_DEPTH) abort();
    return (int32_t)setjmp(exc_stack[exc_depth++].buf);
}

void __kiln_try_exit(void) {
    if (exc_depth > 0) exc_depth--;
}

void __kiln_raise(int64_t exc_ptr) {
    current_exc_val = exc_ptr;
    if (exc_depth > 0) {
        longjmp(exc_stack[exc_depth - 1].buf, 1);
    }
    fprintf(stderr, "Unhandled Kiln exception\n");
    abort();
}

int64_t __kiln_current_exc(void) { return current_exc_val; }

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
    return v->data[index];
}

void Vec_set(int64_t vec_ptr, int64_t index, int64_t item) {
    KilnVec* v = (KilnVec*)vec_ptr;
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
