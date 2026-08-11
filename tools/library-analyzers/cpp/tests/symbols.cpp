// Integration test for the ce-cpp AST symbol analyzer (spec §§6.7, 12.5,
// 13.1; plan 047).
//
// The test loads the checked-in `cpp-symbols-request.json` fixture under the
// fixture tree, runs `analyzeSymbolsForTarget` against each library, folds
// the resulting `SymbolAnalysis` values into a synthetic `AnalysisResponse`,
// and compares the JSON serialization byte-for-byte with
// `cpp-symbols-response.json`. Set `UPDATE_EXPECT=1` to rewrite the
// checked-in fixture from a green run.
//
// In addition to the fixture-driven check, a handful of targeted assertions
// verify kind classification, overload handling, and location coordinates
// so a semantic drift produces a precise error before the byte diff runs.

#include "compile_profile.hpp"
#include "protocol.hpp"
#include "symbols.hpp"

#include <algorithm>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>

namespace fs = std::filesystem;

namespace {

int failures = 0;

#define CE_EXPECT_TRUE(expr)                                                                    \
    do {                                                                                        \
        if (!(expr)) {                                                                          \
            std::cerr << __FILE__ << ":" << __LINE__ << ": expectation failed: " << #expr       \
                      << std::endl;                                                             \
            ++failures;                                                                         \
        }                                                                                       \
    } while (0)

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

std::string chomp(const std::string& s) {
    if (!s.empty() && s.back() == '\n') return s.substr(0, s.size() - 1);
    return s;
}

fs::path tree_root() { return fs::path(CE_CPP_TREE_DIR); }
fs::path protocol_fixture_dir() { return fs::path(CE_CPP_PROTOCOL_FIXTURE_DIR); }
fs::path expected_fixture_path() {
    return protocol_fixture_dir() / "cpp-symbols-response.json";
}

/// Build the in-memory `CompileProfile` the symbol tests need. Adds the
/// symbol fixture directory as an include root so `#include "basic.hpp"`
/// style headers inside the tree resolve without touching the on-disk
/// `compile-profile.toml`.
ce_cpp::CompileProfile build_test_profile() {
    ce_cpp::CompileProfile p;
    p.repository_root = fs::canonical(tree_root());
    p.cxx_standard = "c++20";
    p.include_roots.push_back(fs::canonical(tree_root() / "libraries/cpp/symbols"));
    p.include_roots.push_back(fs::canonical(tree_root() / "libraries/cpp"));
    return p;
}

ce_cpp::LibraryAnalysis run_target(const ce_cpp::CompileProfile& profile,
                                   const std::string& target_path) {
    fs::path source = tree_root() / target_path;
    auto outcome = ce_cpp::analyzeSymbolsForTarget(profile, source, target_path);
    ce_cpp::LibraryAnalysis la;
    la.path = target_path;
    la.dependency_analysis.state = ce_cpp::AnalysisState::Complete;
    la.symbol_analysis = std::move(outcome.analysis);
    return la;
}

std::string serialize_payload(const std::vector<ce_cpp::LibraryAnalysis>& libraries) {
    ce_cpp::AnalysisResponse resp;
    resp.schema_version = ce_cpp::SCHEMA_VERSION;
    resp.adapter.name = std::string(ce_cpp::ADAPTER_NAME);
    resp.adapter.version = std::string(ce_cpp::ADAPTER_VERSION);
    ce_cpp::ToolchainIdentity clang;
    clang.name = "clang";
    clang.version = "22.1.0";
    clang.target = std::string("x86_64-unknown-linux-gnu");
    resp.adapter.toolchains.push_back(std::move(clang));
    resp.libraries = libraries;
    return ce_cpp::serialize_response(resp);
}

const ce_cpp::Symbol* find_symbol(const ce_cpp::SymbolAnalysis& sa, const std::string& name,
                                  const std::string& kind) {
    for (const auto& s : sa.symbols) {
        if (s.name == name && s.kind == kind) return &s;
    }
    return nullptr;
}

size_t count_kind(const ce_cpp::SymbolAnalysis& sa, const std::string& kind) {
    size_t c = 0;
    for (const auto& s : sa.symbols) {
        if (s.kind == kind) ++c;
    }
    return c;
}

void run_semantic_checks(const std::vector<ce_cpp::LibraryAnalysis>& libraries) {
    // Locate each library analysis by its wire path.
    auto get = [&](const std::string& p) -> const ce_cpp::LibraryAnalysis& {
        for (const auto& la : libraries) {
            if (la.path == p) return la;
        }
        std::cerr << "missing library analysis: " << p << std::endl;
        std::abort();
    };

    const auto& basic = get("libraries/cpp/symbols/basic.hpp").symbol_analysis;
    CE_EXPECT_TRUE(basic.state == ce_cpp::AnalysisState::Complete ||
                   basic.state == ce_cpp::AnalysisState::Partial);
    // Namespace and enclosed class-like decls.
    CE_EXPECT_TRUE(find_symbol(basic, "algebra", "type") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "Point", "class") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "Color", "class") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "Bytes", "class") != nullptr);
    // Enum + enumerator.
    CE_EXPECT_TRUE(find_symbol(basic, "Signal", "type") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "Low", "value") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "High", "value") != nullptr);
    // Alias + typedef map to type.
    CE_EXPECT_TRUE(find_symbol(basic, "Coord", "type") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "Weight", "type") != nullptr);
    // Concept.
    CE_EXPECT_TRUE(find_symbol(basic, "Addable", "concept") != nullptr);
    // Template class.
    CE_EXPECT_TRUE(find_symbol(basic, "Optional", "class") != nullptr);
    // Template function + overloaded free functions.
    CE_EXPECT_TRUE(find_symbol(basic, "identity", "function") != nullptr);
    CE_EXPECT_EQ(count_kind(basic, "function"), size_t{3});  // identity + zero + zero
    // Constructor / destructor / method / operator all classify as method.
    CE_EXPECT_TRUE(find_symbol(basic, "Point", "method") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "~Point", "method") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "shifted", "method") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "origin", "method") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "operator+", "method") != nullptr);
    // Value: variable + enumerator.
    CE_EXPECT_TRUE(find_symbol(basic, "PI", "value") != nullptr);
    CE_EXPECT_TRUE(find_symbol(basic, "Red", "value") != nullptr);

    // Overloads: two `zero` free functions, both emitted, both with the same
    // qualified_name but different signatures.
    size_t zero_count = 0;
    for (const auto& s : basic.symbols) {
        if (s.name == "zero" && s.kind == "function") {
            ++zero_count;
            CE_EXPECT_TRUE(s.qualified_name.has_value());
            CE_EXPECT_EQ(*s.qualified_name, std::string("algebra::zero"));
            CE_EXPECT_TRUE(s.signature.has_value());
        }
    }
    CE_EXPECT_EQ(zero_count, size_t{2});

    // Nested + anonymous namespace symbol behavior.
    const auto& nested = get("libraries/cpp/symbols/nested.hpp").symbol_analysis;
    CE_EXPECT_TRUE(find_symbol(nested, "outer", "type") != nullptr);
    CE_EXPECT_TRUE(find_symbol(nested, "inner", "type") != nullptr);
    CE_EXPECT_TRUE(find_symbol(nested, "Inner", "class") != nullptr);
    if (const auto* inner_class = find_symbol(nested, "Inner", "class")) {
        CE_EXPECT_TRUE(inner_class->qualified_name.has_value());
        CE_EXPECT_EQ(*inner_class->qualified_name,
                     std::string("outer::inner::Inner"));
    }
    CE_EXPECT_TRUE(find_symbol(nested, "Hidden", "class") != nullptr);
    if (const auto* hidden = find_symbol(nested, "Hidden", "class")) {
        CE_EXPECT_TRUE(hidden->qualified_name.has_value());
        // Anonymous namespace contributes `(anonymous namespace)` to the
        // qualified name (adapter-private token, not seen by the core).
        CE_EXPECT_TRUE(hidden->qualified_name->find("(anonymous namespace)") !=
                       std::string::npos);
    }
    CE_EXPECT_TRUE(find_symbol(nested, "top", "function") != nullptr);

    // Unicode names: bytes-wise columns and Unicode identifiers pass through.
    const auto& unicode = get("libraries/cpp/symbols/unicode.hpp").symbol_analysis;
    CE_EXPECT_TRUE(find_symbol(unicode, u8"東京", "type") != nullptr);
    CE_EXPECT_TRUE(find_symbol(unicode, u8"街", "class") != nullptr);
    CE_EXPECT_TRUE(find_symbol(unicode, u8"こんにちは", "function") != nullptr);
}

// ─── Task 2: source-location and recovery semantics ────────────────────────

/// Macro spelling/expansion: declarations synthesized inside a macro body
/// have their spelling location inside the macro definition, so the
/// main-file filter suppresses them. Non-macro decls emitted alongside are
/// still returned.
void test_macro_expansions_are_filtered() {
    ce_cpp::CompileProfile profile = build_test_profile();
    auto la = run_target(profile, "libraries/cpp/symbols/macros.hpp");
    const auto& sa = la.symbol_analysis;
    CE_EXPECT_TRUE(find_symbol(sa, "Real", "class") != nullptr);
    // A `CE_DECLARE_STRUCT(FromMacro)` invocation may either project no
    // symbol at all or emit the record at the invocation site — accept
    // both, but never emit at the macro definition line.
    for (const auto& s : sa.symbols) {
        if (s.location.has_value()) {
            CE_EXPECT_TRUE(s.location->start.line > 3);
        }
    }
}

/// CRLF handling: Clang counts `\r\n` as one line terminator, so decls on
/// the physical Nth line report `line = N`, not `N + 1`.
void test_crlf_line_numbers() {
    ce_cpp::CompileProfile profile = build_test_profile();
    auto la = run_target(profile, "libraries/cpp/symbols/crlf.hpp");
    const auto& sa = la.symbol_analysis;
    const auto* first = find_symbol(sa, "First", "class");
    CE_EXPECT_TRUE(first != nullptr);
    if (first != nullptr && first->location.has_value()) {
        CE_EXPECT_EQ(first->location->start.line, uint32_t{3});
    }
    const auto* second = find_symbol(sa, "Second", "class");
    CE_EXPECT_TRUE(second != nullptr);
    if (second != nullptr && second->location.has_value()) {
        CE_EXPECT_EQ(second->location->start.line, uint32_t{6});
    }
}

/// Duplicate declarations + forward declarations: each entity yields at most
/// one emitted symbol. Prefer the definition when one exists in the main
/// file; otherwise the earliest declaration in the main file.
void test_forward_and_duplicate_declarations() {
    ce_cpp::CompileProfile profile = build_test_profile();
    auto la = run_target(profile, "libraries/cpp/symbols/forward.hpp");
    const auto& sa = la.symbol_analysis;
    // `class NotDefined;` — no definition, single forward decl emitted.
    size_t not_def_count = 0;
    for (const auto& s : sa.symbols) {
        if (s.name == "NotDefined") ++not_def_count;
    }
    CE_EXPECT_EQ(not_def_count, size_t{1});
    // `class Defined;` + `class Defined { … };` — one symbol at the
    // definition line.
    size_t defined_count = 0;
    uint32_t defined_line = 0;
    for (const auto& s : sa.symbols) {
        if (s.name == "Defined" && s.kind == "class") {
            ++defined_count;
            if (s.location.has_value()) defined_line = s.location->start.line;
        }
    }
    CE_EXPECT_EQ(defined_count, size_t{1});
    CE_EXPECT_TRUE(defined_line >= 5);
    // Three redeclarations of `plain` collapse to one emit at the definition.
    size_t plain_count = 0;
    uint32_t plain_line = 0;
    for (const auto& s : sa.symbols) {
        if (s.name == "plain" && s.kind == "function") {
            ++plain_count;
            if (s.location.has_value()) plain_line = s.location->start.line;
        }
    }
    CE_EXPECT_EQ(plain_count, size_t{1});
    CE_EXPECT_TRUE(plain_line >= 10);
}

/// Symbols from included headers must not leak into the target's analysis.
/// The target only owns declarations physically inside its own file.
void test_included_header_symbols_not_leaked() {
    ce_cpp::CompileProfile profile = build_test_profile();
    auto la = run_target(profile, "libraries/cpp/symbols/headers.hpp");
    const auto& sa = la.symbol_analysis;
    // Local decls appear.
    CE_EXPECT_TRUE(find_symbol(sa, "LocalOnly", "class") != nullptr);
    CE_EXPECT_TRUE(find_symbol(sa, "local_fn", "function") != nullptr);
    // Included basic.hpp decls must not appear.
    CE_EXPECT_TRUE(find_symbol(sa, "algebra", "type") == nullptr);
    CE_EXPECT_TRUE(find_symbol(sa, "Point", "class") == nullptr);
    CE_EXPECT_TRUE(find_symbol(sa, "Addable", "concept") == nullptr);
}

/// Parse recovery: an error mid-file leaves earlier decls intact and marks
/// the analysis `partial` so the caller can see the catalog is incomplete.
/// Dependency completeness is not tested here — that lives in the usecases
/// integration test — but the state contract must hold.
void test_parse_recovery_is_partial() {
    ce_cpp::CompileProfile profile = build_test_profile();
    auto la = run_target(profile, "libraries/cpp/symbols/recovery.hpp");
    const auto& sa = la.symbol_analysis;
    CE_EXPECT_TRUE(find_symbol(sa, "Before", "class") != nullptr);
    CE_EXPECT_TRUE(find_symbol(sa, "before_fn", "function") != nullptr);
    // Clang's recovery is best-effort — either the analysis state is
    // `partial` (typical) or the driver bailed and we saw `failed`. Either
    // must not be `complete`.
    CE_EXPECT_TRUE(sa.state != ce_cpp::AnalysisState::Complete);
}

void run_task2_checks() {
    test_macro_expansions_are_filtered();
    test_crlf_line_numbers();
    test_forward_and_duplicate_declarations();
    test_included_header_symbols_not_leaked();
    test_parse_recovery_is_partial();
}

void run_test() {
    ce_cpp::CompileProfile profile = build_test_profile();

    std::vector<std::string> targets = {
        "libraries/cpp/symbols/basic.hpp",
        "libraries/cpp/symbols/nested.hpp",
        "libraries/cpp/symbols/unicode.hpp",
    };

    std::vector<ce_cpp::LibraryAnalysis> library_analyses;
    for (const auto& t : targets) {
        library_analyses.push_back(run_target(profile, t));
    }

    run_semantic_checks(library_analyses);
    run_task2_checks();

    std::string actual = serialize_payload(library_analyses);

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
        std::cerr << failures << " C++ symbols assertions failed" << std::endl;
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
