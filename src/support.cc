#include <chrono>
#include <cstdio>
#include <thread>
#include <vector>

static std::vector<std::thread> THREADS;

extern "C" void support_spawn_script(void (*f)()) {
  THREADS.emplace_back(std::move(f));
}

extern "C" void support_detach_scripts() {
  for (auto& t : THREADS) {
    t.detach();
  }
}

extern "C" void support_join_scripts() {
  for (auto& t : THREADS) {
    t.join();
  }
}

extern "C" void support_write_float(double f) {
  printf("%f\n", f);
}

extern "C" void support_sleep(double s) {
  std::this_thread::sleep_for(std::chrono::duration<double, std::ratio<1>>(s));
}
