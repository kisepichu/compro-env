# CMake build for the ce-cpp adapter (spec §§6.7, 6.9; plans 045 & 046).
#
# The caller (usually tools/library-analyzers/cpp/build.sh, invoked by
# `library-adapter-build`) points `LLVM_DIR` at the prepared LLVM 22.1.0
# CMake package directory:
#
#     cmake -S tools/library-analyzers/cpp -B <staging>/cpp \
#         -DCMAKE_BUILD_TYPE=Release \
#         -DLLVM_DIR=<prepared>/archives/llvm-<triple>/<root>/lib/cmake/llvm
#     cmake --build <staging>/cpp --target cpp-analyzer -- -j4
#
# We use `find_package(LLVM)` only to confirm the pinned version and pick up
# `LLVM_INCLUDE_DIRS`. Linking goes through `libclang-cpp.so` directly rather
# than through the `find_package(Clang)` imported targets — the latter drags
# in `ZLIB::ZLIB` and `zstd::libzstd_shared` interface targets that would
# otherwise force every host to install matching CMake packages before the
# adapter can build. `libclang-cpp.so` ships in the pinned LLVM tarball and
# already carries the necessary runtime deps as classic `.so` links.

cmake_minimum_required(VERSION 3.20)

project(ce_cpp_analyzer LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# Refuse to fall back to a host-installed LLVM. `NO_DEFAULT_PATH` restricts the
# search to the caller-provided `LLVM_DIR`, and `EXACT` locks the version.
if(NOT DEFINED LLVM_DIR)
    message(FATAL_ERROR
        "LLVM_DIR must point at the prepared LLVM 22.1.0 CMake package "
        "directory (e.g. <prepared>/archives/llvm-linux-x64/<root>/lib/cmake/llvm)")
endif()

find_package(LLVM 22.1.0 EXACT REQUIRED CONFIG
    PATHS ${LLVM_DIR}
    NO_DEFAULT_PATH)
message(STATUS "ce-cpp: found LLVM ${LLVM_PACKAGE_VERSION} at ${LLVM_INSTALL_PREFIX}")

# Locate `libclang-cpp.so` inside the same prepared tree. `LLVM_INSTALL_PREFIX`
# is set by `find_package(LLVM)` above and points at the root of the tarball.
find_library(CE_CLANG_CPP_LIBRARY clang-cpp
    PATHS ${LLVM_INSTALL_PREFIX}/lib
    NO_DEFAULT_PATH
    REQUIRED)
message(STATUS "ce-cpp: linking against ${CE_CLANG_CPP_LIBRARY}")

# libclang-cpp.so has an unversioned SONAME on some LLVM builds; embed an
# absolute RPATH so the launched executable finds it regardless of
# `LD_LIBRARY_PATH`. We only add the LLVM `lib/` directory — nothing else
# from the prepared tree leaks into the runtime search path.
set(CMAKE_INSTALL_RPATH "${LLVM_INSTALL_PREFIX}/lib")
set(CMAKE_BUILD_RPATH "${LLVM_INSTALL_PREFIX}/lib")
set(CMAKE_BUILD_WITH_INSTALL_RPATH TRUE)

add_library(ce_cpp_protocol STATIC src/protocol.cpp)
target_include_directories(ce_cpp_protocol
    PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include)

add_library(ce_cpp_compile_profile STATIC src/compile_profile.cpp)
target_include_directories(ce_cpp_compile_profile
    PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include)

# The dependency analyzer needs Clang headers plus the monolithic
# `libclang-cpp.so`. Passing the library path directly (instead of via
# `find_package(Clang)`) sidesteps the exported CMake target's dependency
# on optional system packages (ZLIB, zstd, LibXml2, ...).
#
# LLVM/Clang ship compiled with `-fno-rtti`; deriving from `PPCallbacks` /
# `PreprocessOnlyAction` requires our translation unit to also disable RTTI
# so the linker doesn't demand typeinfo the shared library never emits.
add_library(ce_cpp_dependencies STATIC src/dependencies.cpp)
target_include_directories(ce_cpp_dependencies
    PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include
    PRIVATE ${LLVM_INCLUDE_DIRS})
target_compile_options(ce_cpp_dependencies PRIVATE -fno-rtti)
target_link_libraries(ce_cpp_dependencies
    PUBLIC ce_cpp_protocol ce_cpp_compile_profile
    PRIVATE ${CE_CLANG_CPP_LIBRARY})

add_executable(cpp-analyzer src/main.cpp)
target_include_directories(cpp-analyzer PRIVATE ${LLVM_INCLUDE_DIRS})
target_link_libraries(cpp-analyzer PRIVATE
    ce_cpp_protocol
    ce_cpp_compile_profile
    ce_cpp_dependencies)
# `libclang-cpp.so` transitively depends on `libz.so.1` and `libstdc++.so.6`,
# which the pinned LLVM tarball assumes the host provides. `--allow-shlib-
# undefined` lets the analyzer link on hosts (e.g., Nix builders) that lack
# a globally-installed `libz` at cmake time — the loader resolves the actual
# shared-library reference at runtime from the process environment.
target_link_options(cpp-analyzer PRIVATE
    "LINKER:--allow-shlib-undefined")

# ─── Tests ──────────────────────────────────────────────────────────────────

enable_testing()

add_executable(handshake_test tests/handshake.cpp)
target_link_libraries(handshake_test PRIVATE ce_cpp_protocol)
target_compile_definitions(handshake_test PRIVATE
    CE_CPP_FIXTURE_DIR="${CMAKE_CURRENT_SOURCE_DIR}/../protocol/fixtures")

add_test(NAME handshake COMMAND handshake_test)

add_executable(compile_profile_test tests/compile_profile.cpp)
target_link_libraries(compile_profile_test PRIVATE ce_cpp_compile_profile)

add_test(NAME compile_profile COMMAND compile_profile_test)

add_executable(dependencies_test tests/dependencies.cpp)
target_link_libraries(dependencies_test PRIVATE
    ce_cpp_protocol
    ce_cpp_compile_profile
    ce_cpp_dependencies)
target_link_options(dependencies_test PRIVATE
    "LINKER:--allow-shlib-undefined")
target_compile_definitions(dependencies_test PRIVATE
    CE_CPP_TREE_DIR="${CMAKE_CURRENT_SOURCE_DIR}/tests/fixtures/tree"
    CE_CPP_PROTOCOL_FIXTURE_DIR="${CMAKE_CURRENT_SOURCE_DIR}/../protocol/fixtures")

add_test(NAME dependencies COMMAND dependencies_test)
