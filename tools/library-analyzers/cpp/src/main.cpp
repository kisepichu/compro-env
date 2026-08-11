// `ce-cpp` — C++ language analyzer for compro-env (spec §§6.7, 6.9;
// plans 045 & 046).
//
// Reads one `AnalysisRequest` JSON document from stdin, checks the protocol
// version, then for each library/solution target runs a Clang
// preprocess-only pass over the target source, collects direct include
// dependencies, and writes an `AnalysisResponse` JSON document to stdout.
//
// The observed toolchain is derived from `<llvm/Config/llvm-config.h>`
// preprocessor macros — that header ships in the exact prepared LLVM 22.1.0
// tree and can never disagree with the linked libraries.

#include <llvm/Config/llvm-config.h>

#include <cstdlib>
#include <exception>
#include <filesystem>
#include <iostream>
#include <iterator>
#include <sstream>
#include <string>

#include "compile_profile.hpp"
#include "dependencies.hpp"
#include "protocol.hpp"

namespace fs = std::filesystem;

namespace {

#define CE_CPP_STRINGIFY_INNER(x) #x
#define CE_CPP_STRINGIFY(x) CE_CPP_STRINGIFY_INNER(x)

std::string read_all_stdin() {
    std::cin >> std::noskipws;
    std::istreambuf_iterator<char> begin(std::cin);
    std::istreambuf_iterator<char> end;
    return std::string(begin, end);
}

ce_cpp::AdapterIdentity make_adapter_identity() {
    ce_cpp::AdapterIdentity adapter;
    adapter.name = std::string(ce_cpp::ADAPTER_NAME);
    adapter.version = std::string(ce_cpp::ADAPTER_VERSION);
    const std::string clang_version =
        CE_CPP_STRINGIFY(LLVM_VERSION_MAJOR) "." CE_CPP_STRINGIFY(LLVM_VERSION_MINOR) "." CE_CPP_STRINGIFY(
            LLVM_VERSION_PATCH);
    ce_cpp::ToolchainIdentity clang;
    clang.name = "clang";
    clang.version = clang_version;
#ifdef LLVM_DEFAULT_TARGET_TRIPLE
    clang.target = std::string(LLVM_DEFAULT_TARGET_TRIPLE);
#endif
    adapter.toolchains.push_back(std::move(clang));
    return adapter;
}

/// Build the manifest set out of the request's `libraries[]` entries. The
/// manifest is used to classify each include as internal / external /
/// unresolved (see `dependencies.hpp`).
ce_cpp::ManifestSet build_manifest(const ce_cpp::AnalysisRequest& req) {
    ce_cpp::ManifestSet out;
    out.reserve(req.libraries.size());
    for (const auto& lib : req.libraries) {
        out.push_back(lib.path);
    }
    return out;
}

ce_cpp::AnalysisResponse build_response(const ce_cpp::AnalysisRequest& request) {
    ce_cpp::AnalysisResponse resp;
    resp.schema_version = ce_cpp::SCHEMA_VERSION;
    resp.adapter = make_adapter_identity();

    // Empty request (used for the plan-045 handshake): nothing to analyze,
    // return the identity envelope only.
    if (request.libraries.empty() && request.solutions.empty()) {
        return resp;
    }

    ce_cpp::CompileProfile profile =
        ce_cpp::loadCompileProfile(fs::path(request.repository_root));

    ce_cpp::ManifestSet manifest = build_manifest(request);
    for (const auto& lib : request.libraries) {
        fs::path source = fs::path(request.repository_root) / lib.path;
        auto outcome =
            ce_cpp::analyze_target(profile, source, lib.path, manifest);
        ce_cpp::LibraryAnalysis la;
        la.path = lib.path;
        la.dependency_analysis.dependencies = std::move(outcome.dependencies);
        la.dependency_analysis.state = outcome.state;
        // Symbol analysis lands in plan 047; return an empty partial set.
        la.symbol_analysis.state = ce_cpp::AnalysisState::Partial;
        la.diagnostics = std::move(outcome.diagnostics);
        resp.libraries.push_back(std::move(la));
    }
    for (const auto& sol : request.solutions) {
        fs::path source = fs::path(request.repository_root) / sol.root / sol.entry;
        std::string entry_relative = (fs::path(sol.root) / sol.entry).generic_string();
        auto outcome =
            ce_cpp::analyze_target(profile, source, entry_relative, manifest);
        ce_cpp::SolutionAnalysis sa;
        sa.id = sol.id;
        sa.dependency_analysis.dependencies = std::move(outcome.dependencies);
        sa.dependency_analysis.state = outcome.state;
        sa.diagnostics = std::move(outcome.diagnostics);
        resp.solutions.push_back(std::move(sa));
    }
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
