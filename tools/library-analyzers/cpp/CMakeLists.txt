# CMake build for the ce-cpp adapter (spec §§6.7, 6.9; plan 045 Task 2).
#
# The caller (usually tools/library-analyzers/cpp/build.sh, invoked by
# `library-adapter-build`) points `LLVM_DIR` (and optionally `Clang_DIR`) at
# the prepared LLVM 22.1.0 CMake package directories:
#
#     cmake -S tools/library-analyzers/cpp -B <staging>/cpp \
#         -DCMAKE_BUILD_TYPE=Release \
#         -DLLVM_DIR=<prepared>/archives/llvm-<triple>/<root>/lib/cmake/llvm \
#         -DClang_DIR=<prepared>/archives/llvm-<triple>/<root>/lib/cmake/clang
#     cmake --build <staging>/cpp --target cpp-analyzer -- -j4
#
# We only use LLVM's include tree to read `<llvm/Config/llvm-config.h>`; no
# LLVM library is linked. That keeps the handshake executable free of a
# shared-library runtime dependency on the prepared LLVM tree, so `bin/cpp-
# analyzer` can be published without also stitching in `LD_LIBRARY_PATH`.

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

if(DEFINED Clang_DIR)
    find_package(Clang REQUIRED CONFIG
        PATHS ${Clang_DIR}
        NO_DEFAULT_PATH)
endif()

add_library(ce_cpp_protocol STATIC src/protocol.cpp)
target_include_directories(ce_cpp_protocol
    PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include)

add_library(ce_cpp_compile_profile STATIC src/compile_profile.cpp)
target_include_directories(ce_cpp_compile_profile
    PUBLIC ${CMAKE_CURRENT_SOURCE_DIR}/include)

add_executable(cpp-analyzer src/main.cpp)
target_include_directories(cpp-analyzer PRIVATE ${LLVM_INCLUDE_DIRS})
target_link_libraries(cpp-analyzer PRIVATE ce_cpp_protocol ce_cpp_compile_profile)

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
