cmake_minimum_required(VERSION 3.20)
project(xex2_python LANGUAGES CXX)

# ──────────────────────────────────────────────────────────────────────────
# Dependencies
# ──────────────────────────────────────────────────────────────────────────
find_package(Python 3.10 COMPONENTS Interpreter Development.Module REQUIRED)
find_package(nanobind CONFIG REQUIRED)

# ──────────────────────────────────────────────────────────────────────────
# Paths into the workspace
# ──────────────────────────────────────────────────────────────────────────
set(WORKSPACE_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../..")
set(XEX2_FFI_LIB_DIR "${WORKSPACE_ROOT}/target/release")
set(XEX2_FFI_CPP_INCLUDE "${WORKSPACE_ROOT}/crates/xex2-ffi/bindings/cpp")

# Sanity check: we need the diplomat-generated headers in place. These are
# produced by `diplomat-tool cpp crates/xex2-ffi/bindings/cpp --entry crates/xex2-ffi/src/lib.rs`
# and gitignored. CI / a build script should run diplomat-tool before `uv build`.
if(NOT EXISTS "${XEX2_FFI_CPP_INCLUDE}/Xex2.hpp")
    message(FATAL_ERROR
        "Diplomat-generated headers not found at ${XEX2_FFI_CPP_INCLUDE}.\n"
        "Run:\n"
        "  diplomat-tool cpp crates/xex2-ffi/bindings/cpp --entry crates/xex2-ffi/src/lib.rs\n"
        "from the workspace root first.")
endif()

# Sanity check: the xex2-ffi cdylib/staticlib must already be built.
# On macOS we link against libxex2_ffi.dylib; on Linux .so; on Windows
# .dll.lib. Run `cargo build -p xex2-ffi --release` from the workspace
# root before invoking the wheel build.
if(APPLE)
    set(XEX2_FFI_LIB "${XEX2_FFI_LIB_DIR}/libxex2_ffi.dylib")
elseif(WIN32)
    set(XEX2_FFI_LIB "${XEX2_FFI_LIB_DIR}/xex2_ffi.dll.lib")
else()
    set(XEX2_FFI_LIB "${XEX2_FFI_LIB_DIR}/libxex2_ffi.so")
endif()
if(NOT EXISTS "${XEX2_FFI_LIB}")
    message(FATAL_ERROR
        "xex2-ffi library not found at ${XEX2_FFI_LIB}.\n"
        "Run `cargo build -p xex2-ffi --release` from the workspace root first.")
endif()

# ──────────────────────────────────────────────────────────────────────────
# The nanobind extension module
# ──────────────────────────────────────────────────────────────────────────
nanobind_add_module(xex2 NB_STATIC src/xex2.cpp)
target_compile_features(xex2 PRIVATE cxx_std_20)
target_include_directories(xex2 PRIVATE "${XEX2_FFI_CPP_INCLUDE}")
target_link_libraries(xex2 PRIVATE "${XEX2_FFI_LIB}")

# macOS: embed the dylib name as an @rpath load command + tell the loader
# to look alongside the compiled module. For production distribution you'd
# want to use `install_name_tool` / a static build; this is enough for
# local development and `uv run python -c "import xex2"`.
if(APPLE)
    set_target_properties(xex2 PROPERTIES
        BUILD_RPATH "${XEX2_FFI_LIB_DIR}"
        INSTALL_RPATH "${XEX2_FFI_LIB_DIR}")
elseif(UNIX)
    set_target_properties(xex2 PROPERTIES
        BUILD_RPATH "${XEX2_FFI_LIB_DIR}"
        INSTALL_RPATH "${XEX2_FFI_LIB_DIR}")
endif()

if(MSVC)
    target_compile_options(xex2 PRIVATE /Zc:__cplusplus)
endif()

install(TARGETS xex2 LIBRARY DESTINATION .)
