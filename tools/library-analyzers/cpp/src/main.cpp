// `ce-cpp` — C++ language analyzer for compro-env (spec §§6.7, 6.9; plan 045
// Task 2).
//
// Reads one `AnalysisRequest` JSON document from stdin, checks the protocol
// version, reports adapter identity plus the compile-time Clang/LLVM version
// this executable was linked against, and writes one `AnalysisResponse` JSON
// document to stdout. Dependency and symbol extraction land in later plans.
//
// The observed toolchain is derived from `<llvm/Config/llvm-config.h>`
// preprocessor macros — that header ships in the exact prepared LLVM 22.1.0
// tree and can never disagree with the linked libraries. Runtime queries
// (`clang --version`, `llvm-config --version`) are avoided so the reported
// version cannot drift from what CMake actually configured against.

#include <llvm/Config/llvm-config.h>

#include <cstdlib>
#include <exception>
#include <iostream>
#include <iterator>
#include <sstream>
#include <string>

#include "protocol.hpp"

namespace {

/// Stringify a preprocessor macro. The two-step indirection is required so the
/// argument itself is expanded before being turned into a string literal.
#define CE_CPP_STRINGIFY_INNER(x) #x
#define CE_CPP_STRINGIFY(x) CE_CPP_STRINGIFY_INNER(x)

std::string read_all_stdin() {
    std::cin >> std::noskipws;
    std::istreambuf_iterator<char> begin(std::cin);
    std::istreambuf_iterator<char> end;
    return std::string(begin, end);
}

ce_cpp::AnalysisResponse build_response(const ce_cpp::AnalysisRequest& /*request*/) {
    ce_cpp::AnalysisResponse resp;
    resp.schema_version = ce_cpp::SCHEMA_VERSION;
    resp.adapter.name = std::string(ce_cpp::ADAPTER_NAME);
    resp.adapter.version = std::string(ce_cpp::ADAPTER_VERSION);

    // Compile-time LLVM version. spec §6.7 requires exactly 22.1.0; the pinned
    // headers guarantee it. `llvm-config` at runtime is deliberately not
    // consulted because a host `PATH` could shadow the pinned one.
    const std::string clang_version =
        CE_CPP_STRINGIFY(LLVM_VERSION_MAJOR) "." CE_CPP_STRINGIFY(LLVM_VERSION_MINOR) "." CE_CPP_STRINGIFY(
            LLVM_VERSION_PATCH);

    ce_cpp::ToolchainIdentity clang;
    clang.name = "clang";
    clang.version = clang_version;
#ifdef LLVM_DEFAULT_TARGET_TRIPLE
    clang.target = std::string(LLVM_DEFAULT_TARGET_TRIPLE);
#endif
    resp.adapter.toolchains.push_back(std::move(clang));
    return resp;
}

}  // namespace

int main() {
    try {
        std::string raw = read_all_stdin();
        ce_cpp::AnalysisRequest req = ce_cpp::parse_request(raw);
        ce_cpp::AnalysisResponse resp = build_response(req);
        std::string out = ce_cpp::serialize_response(resp);
        std::cout << out;
        std::cout.flush();
        return EXIT_SUCCESS;
    } catch (const std::exception& e) {
        std::cerr << "ce-cpp: error: " << e.what() << std::endl;
        return EXIT_FAILURE;
    }
}
