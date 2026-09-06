cmake_minimum_required(VERSION 3.16)
project(vantage_sim LANGUAGES C CXX)

set(CMAKE_C_STANDARD 99)
set(CMAKE_CXX_STANDARD 14)

find_package(SDL2 REQUIRED)

# Tell the vendored LVGL to use sim/lv_conf.h and build for desktop.
set(LV_BUILD_CONF_PATH "${CMAKE_CURRENT_SOURCE_DIR}/lv_conf.h" CACHE PATH
    "Path to the simulator lv_conf.h" FORCE)
set(CONFIG_LV_BUILD_DEMOS OFF CACHE BOOL "" FORCE)
set(CONFIG_LV_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)

add_subdirectory(
  "${CMAKE_CURRENT_SOURCE_DIR}/../components/lvgl"
  "${CMAKE_CURRENT_BINARY_DIR}/lvgl")

add_executable(vantage_sim
  main.c
  wm.c
  "${CMAKE_CURRENT_SOURCE_DIR}/../main/wm_anim.c"
)

target_include_directories(vantage_sim PRIVATE
  "${CMAKE_CURRENT_SOURCE_DIR}"
  "${CMAKE_CURRENT_SOURCE_DIR}/../main"
)

target_link_libraries(vantage_sim PRIVATE lvgl SDL2::SDL2)
