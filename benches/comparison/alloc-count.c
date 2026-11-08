#include <stdatomic.h>
#include <stddef.h>
#include <stdio.h>

extern void *__libc_malloc(size_t);
extern void *__libc_calloc(size_t, size_t);
extern void *__libc_realloc(void *, size_t);
extern void *__libc_memalign(size_t, size_t);
extern void __libc_free(void *);

#define BUCKETS 16385
static atomic_ulong sizes[BUCKETS];

static void count(size_t size) {
  atomic_fetch_add_explicit(&sizes[size < BUCKETS ? size : BUCKETS - 1], 1, memory_order_relaxed);
}

void *malloc(size_t size) { count(size); return __libc_malloc(size); }
void *calloc(size_t n, size_t size) { count(n * size); return __libc_calloc(n, size); }
void *realloc(void *p, size_t size) { count(size); return __libc_realloc(p, size); }
void *aligned_alloc(size_t align, size_t size) { count(size); return __libc_memalign(align, size); }
int posix_memalign(void **p, size_t align, size_t size) {
  count(size);
  *p = __libc_memalign(align, size);
  return *p ? 0 : 12;
}
void free(void *p) { __libc_free(p); }

__attribute__((destructor)) static void report(void) {
  for (size_t i = 0; i < BUCKETS; i++) {
    unsigned long n = atomic_load(&sizes[i]);
    if (n) fprintf(stderr, "SIZE %zu %lu\n", i, n);
  }
}
