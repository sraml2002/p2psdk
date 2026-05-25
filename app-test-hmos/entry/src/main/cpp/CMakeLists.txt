cmake_minimum_required(VERSION 3.5.1)
project(ppsdk)

# Import pre-built Rust NAPI library (built by build-so.sh).
# This CMakeLists.txt exists solely so the build system discovers
# type declarations in cpp/types/libppsdk/index.d.ts.
add_library(ppsdk SHARED IMPORTED GLOBAL)
set_target_properties(ppsdk PROPERTIES
    IMPORTED_LOCATION "${CMAKE_CURRENT_SOURCE_DIR}/../libs/${OHOS_ARCH}/libppsdk.so"
)
