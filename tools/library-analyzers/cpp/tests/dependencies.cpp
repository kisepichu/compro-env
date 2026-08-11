// Integration test for the ce-cpp direct-dependency analyzer (spec §6.7;
// plan 046 Task 2).
//
// The test loads the checked-in `cpp-dependencies-request.json` fixture
// under the fixture tree, runs `analyze_target` against each library and
// solution entry, and compares the JSON-serialized output against
// `cpp-dependencies-response.json` byte-for-byte after normalizing the
// trailing newline. Any semantic drift surfaces as a diff.
//
// Set `UPDATE_EXPECT=1` to overwrite the checked-in response fixture with
// the analyzer's current output — useful when adding a new fixture edge.

#include "compile_profile.hpp"
#include "dependencies.hpp"
#include "protocol.hpp"

#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>

namespace fs = std::filesystem;

namespace {

int failures = 0;

#define CE_EXPECT_EQ(a, b)                                                                    \
    do {                                                                                      \
        auto _a = (a);                                                                        \
        auto _b = (b);                                                                        \
        if (!(_a == _b)) {                                                                    \
            std::cerr << __FILE__ << ":" << __LINE__ << ": expected equality " << #a << " == "\
                      << #b << std::endl;                                                     \
            ++failures;                                                                       \
        }                                                                                     \
    } while (0)

std::string read_file(const fs::path& path) {
    std::ifstream f(path);
    if (!f.is_open()) {
        std::cerr << "failed to open fixture: " << path << std::endl;
        std::abort();
    }
    std::ostringstream ss;
    ss << f.rdbuf();
    return ss.str();
}

void write_file(const fs::path& path, const std::string& contents) {
    std::ofstream f(path);
    if (!f.is_open()) {
        std::cerr << "failed to open fixture for write: " << path << std::endl;
        std::abort();
    }
    f << contents;
}

/// Trim a single trailing newline so JSON files that end with `\n` compare
/// equal to serializer output that does not.
std::string chomp(const std::string& s) {
    if (!s.empty() && s.back() == '\n') return s.substr(0, s.size() - 1);
    return s;
}

fs::path tree_root() { return fs::path(CE_CPP_TREE_DIR); }
fs::path protocol_fixture_dir() { return fs::path(CE_CPP_PROTOCOL_FIXTURE_DIR); }
fs::path expected_fixture_path() {
    return protocol_fixture_dir() / "cpp-dependencies-response.json";
}

/// The fixture tree does not carry a `compile-profile.toml` file — the test
/// builds one in memory by pointing the include root at
/// `<tree>/libraries/cpp` (for quoted `#include "…"`) and a sibling
/// `../external-includes/` directory (for angle-bracketed `#include <…>`).
/// The external directory lives outside `repository_root`, so anything
/// resolved through it is classified as `external`.
ce_cpp::CompileProfile build_test_profile() {
    ce_cpp::CompileProfile p;
    p.repository_root = fs::canonical(tree_root());
    p.cxx_standard = "c++20";
    p.include_roots.push_back(fs::canonical(tree_root() / "libraries/cpp"));
    p.include_roots.push_back(
        fs::canonical(fs::path(CE_CPP_TREE_DIR).parent_path() / "external-includes"));
    return p;
}

/// Manifest set advertised by the wire request. `d.hpp` is included so the
/// macro-expanded `#include INC_D` in `a.hpp` classifies as an internal
/// edge rather than an unresolved one.
ce_cpp::ManifestSet manifest_set() {
    return {
        "libraries/cpp/a.hpp",
        "libraries/cpp/b.hpp",
        "libraries/cpp/c.hpp",
        "libraries/cpp/d.hpp",
        "libraries/cpp/日本語.hpp",
    };
}

void write_library_analysis_field(ce_cpp::LibraryAnalysis& la,
                                  const std::string& target_path,
                                  ce_cpp::TargetOutcome outcome) {
    la.path = target_path;
    la.dependency_analysis.dependencies = std::move(outcome.dependencies);
    la.dependency_analysis.state = outcome.state;
    la.symbol_analysis.state = ce_cpp::AnalysisState::Partial;
    la.diagnostics = std::move(outcome.diagnostics);
}

/// Wire-shape "libraries + solutions" sub-response. The header of the full
/// `AnalysisResponse` — `schema_version` + `adapter` — is decided by the
/// binary at runtime; here we compare only the analytic payload.
std::string serialize_payload(const std::vector<ce_cpp::LibraryAnalysis>& libraries,
                              const std::vector<ce_cpp::SolutionAnalysis>& solutions) {
    ce_cpp::AnalysisResponse resp;
    resp.schema_version = ce_cpp::SCHEMA_VERSION;
    resp.adapter.name = std::string(ce_cpp::ADAPTER_NAME);
    resp.adapter.version = std::string(ce_cpp::ADAPTER_VERSION);
    // Toolchain identity is timing-sensitive (LLVM version macro); pin it
    // in the fixture so byte comparison stays stable.
    ce_cpp::ToolchainIdentity clang;
    clang.name = "clang";
    clang.version = "22.1.0";
    clang.target = std::string("x86_64-unknown-linux-gnu");
    resp.adapter.toolchains.push_back(std::move(clang));
    resp.libraries = libraries;
    resp.solutions = solutions;
    return ce_cpp::serialize_response(resp);
}

void run_test() {
    ce_cpp::CompileProfile profile = build_test_profile();
    ce_cpp::ManifestSet manifest = manifest_set();

    std::vector<std::pair<std::string, fs::path>> library_targets = {
        {"libraries/cpp/a.hpp", tree_root() / "libraries/cpp/a.hpp"},
        {"libraries/cpp/b.hpp", tree_root() / "libraries/cpp/b.hpp"},
        {"libraries/cpp/c.hpp", tree_root() / "libraries/cpp/c.hpp"},
        {"libraries/cpp/d.hpp", tree_root() / "libraries/cpp/d.hpp"},
        {"libraries/cpp/日本語.hpp", tree_root() / "libraries/cpp/日本語.hpp"},
    };

    std::vector<ce_cpp::LibraryAnalysis> library_analyses;
    for (const auto& [target_path, source] : library_targets) {
        auto outcome =
            ce_cpp::analyze_target(profile, source, target_path, manifest);
        ce_cpp::LibraryAnalysis la;
        write_library_analysis_field(la, target_path, std::move(outcome));
        library_analyses.push_back(std::move(la));
    }

    ce_cpp::SolutionAnalysis solution_analysis;
    {
        auto outcome = ce_cpp::analyze_target(
            profile, tree_root() / "solutions/abc/A/main/src/main.cpp",
            "solutions/abc/A/main/src/main.cpp", manifest);
        solution_analysis.id = "abc/A/main";
        solution_analysis.dependency_analysis.dependencies =
            std::move(outcome.dependencies);
        solution_analysis.dependency_analysis.state = outcome.state;
        solution_analysis.diagnostics = std::move(outcome.diagnostics);
    }

    std::string actual =
        serialize_payload(library_analyses, {solution_analysis});

    if (std::getenv("UPDATE_EXPECT") != nullptr) {
        write_file(expected_fixture_path(), actual + "\n");
        std::cerr << "wrote fixture to " << expected_fixture_path() << std::endl;
        return;
    }

    std::string expected = chomp(read_file(expected_fixture_path()));

    if (actual != expected) {
        std::cerr << "response fixture drift.\n\n--- expected ---\n"
                  << expected << "\n\n--- actual ---\n"
                  << actual << "\n";
        ++failures;
    }
}

}  // namespace

int main() {
    run_test();
    if (failures > 0) {
        std::cerr << failures << " C++ dependencies assertions failed" << std::endl;
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
