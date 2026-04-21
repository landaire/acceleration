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

# ──────────────────────────────────────────────────────────────────────────
# Pre-flight: headers and library need to be produced *before* CMake runs.
# CI (and the local build recipe in the README) invokes:
#
#   cargo build -p xex2-ffi --release
#   diplomat-tool cpp crates/xex2-ffi/bindings/cpp --entry crates/xex2-ffi/src/lib.rs
#
# both at the workspace root before `pip install` / `uv build` here.
# ──────────────────────────────────────────────────────────────────────────
if(NOT EXISTS "${XEX2_FFI_CPP_INCLUDE}/Xex2.hpp")
    message(FATAL_ERROR
        "Diplomat-generated headers not found at ${XEX2_FFI_CPP_INCLUDE}.\n"
        "Run `diplomat-tool cpp crates/xex2-ffi/bindings/cpp --entry crates/xex2-ffi/src/lib.rs`\n"
        "from the workspace root first.")
endif()

# We link the Rust FFI *statically* (libxex2_ffi.a) into the extension so
# wheels published to PyPI are self-contained. cargo emits both .a and a
# dylib because the crate's `crate-type` is `["cdylib", "staticlib"]`;
# here we only need the static archive.
if(MSVC)
    set(XEX2_FFI_LIB "${XEX2_FFI_LIB_DIR}/xex2_ffi.lib")
else()
    set(XEX2_FFI_LIB "${XEX2_FFI_LIB_DIR}/libxex2_ffi.a")
endif()
if(NOT EXISTS "${XEX2_FFI_LIB}")
    message(FATAL_ERROR
        "xex2-ffi static library not found at ${XEX2_FFI_LIB}.\n"
        "Run `cargo build -p xex2-ffi --release` from the workspace root first.")
endif()

# ──────────────────────────────────────────────────────────────────────────
# Nanobind extension module
# ──────────────────────────────────────────────────────────────────────────
nanobind_add_module(xex2 NB_STATIC src/xex2.cpp)
target_compile_features(xex2 PRIVATE cxx_std_20)
target_include_directories(xex2 PRIVATE "${XEX2_FFI_CPP_INCLUDE}")
target_link_libraries(xex2 PRIVATE "${XEX2_FFI_LIB}")

# ──────────────────────────────────────────────────────────────────────────
# System libraries that Rust's static archive implicitly depends on.
# These come from `cargo rustc --print native-static-libs`; platform-
# specific because each target has its own stdlib needs.
# ──────────────────────────────────────────────────────────────────────────
if(APPLE)
    # CoreFoundation/Security turn up when the crate graph pulls in any
    # TLS or security-adjacent Rust code; we don't hit that today but
    # link them anyway so adding e.g. `rustls` later is painless.
    target_link_libraries(xex2 PRIVATE "-framework CoreFoundation" "-framework Security")
    target_link_libraries(xex2 PRIVATE iconv)
elseif(UNIX)
    # Linux / other unices.
    target_link_libraries(xex2 PRIVATE pthread dl m)
elseif(MSVC)
    target_link_libraries(xex2 PRIVATE bcrypt userenv advapi32 ws2_32 ntdll)
    target_compile_options(xex2 PRIVATE /Zc:__cplusplus)
endif()

install(TARGETS xex2 LIBRARY DESTINATION .)
