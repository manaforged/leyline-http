#include <cstddef>
#include <cstdlib>

#define LEYLINE_CXX_RUNTIME __attribute__((weak, visibility("hidden")))

LEYLINE_CXX_RUNTIME void operator delete(void *ptr, std::size_t) noexcept {
  std::free(ptr);
}

namespace std {
LEYLINE_CXX_RUNTIME void __throw_out_of_range_fmt(const char *, ...) {
  std::abort();
}
}  // namespace std
