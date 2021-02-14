#include <vector>
#include <thread>
#include <cstdio>

static std::vector<std::thread> THREADS;

extern "C" void spawn_script(void(*f)()) {
  THREADS.emplace_back(std::move(f));
}

extern "C" void detach_scripts() {
  for (auto& t : THREADS) {
    t.detach();
  }
}

extern "C" void join_scripts() {
  for (auto& t : THREADS) {
    t.join();
  }
}

extern "C" void write_float(double f) {
  printf("%f\n", f);
}
