// Compile-profile loader for the ce-cpp analyzer (spec §6.7; plan 046 Task 1).
//
// This module parses a strict subset of TOML sufficient for the checked-in
// `compile-profile.toml`:
//
//   * top-level key/value pairs only (no `[section]` headers, no inline
//     tables, no dotted keys, no arrays of tables);
//   * three required keys: `cxx_standard` (string), `defines` (array of
//     strings), `include_roots` (array of strings);
//   * `#` line comments and blank lines between entries;
//   * double-quoted strings with the same `\"` / `\\` escapes we accept
//     elsewhere — no `\n`, `\t`, or Unicode escapes.
//
// Anything outside this subset raises `CompileProfileError`. That intentional
// narrowness keeps the parser small and matches the strict handshake JSON
// parser next door.

#include "compile_profile.hpp"

#include <cctype>
#include <fstream>
#include <optional>
#include <sstream>
#include <string>
#include <string_view>
#include <system_error>
#include <utility>

namespace fs = std::filesystem;

namespace ce_cpp {

namespace {

// ─── Tokenizer / parser helpers ─────────────────────────────────────────────

/// Byte-level cursor over the TOML source.
class Tokenizer {
   public:
    explicit Tokenizer(std::string_view raw) : raw_(raw), pos_(0) {}

    bool eof() const { return pos_ >= raw_.size(); }

    /// Skip spaces, tabs, and `# ...` comments up to (but not including) the
    /// next newline. Does not consume newlines — the caller uses them to
    /// separate key/value entries.
    void skip_line_ws() {
        while (!eof()) {
            char c = raw_[pos_];
            if (c == ' ' || c == '\t') {
                ++pos_;
            } else if (c == '#') {
                while (!eof() && raw_[pos_] != '\n') {
                    ++pos_;
                }
            } else {
                break;
            }
        }
    }

    /// Skip whitespace, comments, and newlines.
    void skip_full_ws() {
        while (!eof()) {
            skip_line_ws();
            if (!eof() && (raw_[pos_] == '\n' || raw_[pos_] == '\r')) {
                ++pos_;
            } else {
                break;
            }
        }
    }

    char peek() const { return raw_[pos_]; }
    char consume() { return raw_[pos_++]; }

    /// Read a bare identifier: `[A-Za-z_][A-Za-z0-9_]*`. Anything else raises.
    std::string parse_bare_key() {
        std::string out;
        if (eof()) {
            throw CompileProfileError("expected key, found end of input");
        }
        char first = raw_[pos_];
        if (!(std::isalpha(static_cast<unsigned char>(first)) || first == '_')) {
            std::ostringstream oss;
            oss << "invalid key start character: '" << first << "'";
            throw CompileProfileError(oss.str());
        }
        while (!eof()) {
            char c = raw_[pos_];
            if (std::isalnum(static_cast<unsigned char>(c)) || c == '_') {
                out.push_back(c);
                ++pos_;
            } else {
                break;
            }
        }
        return out;
    }

    /// Parse a double-quoted string literal. Only `\\` and `\"` escapes are
    /// permitted; anything else raises.
    std::string parse_string() {
        if (eof() || raw_[pos_] != '"') {
            throw CompileProfileError("expected quoted string");
        }
        ++pos_;
        std::string out;
        while (true) {
            if (eof()) {
                throw CompileProfileError("unterminated string literal");
            }
            char c = raw_[pos_++];
            if (c == '"') {
                return out;
            }
            if (c == '\\') {
                if (eof()) {
                    throw CompileProfileError("dangling backslash in string");
                }
                char esc = raw_[pos_++];
                if (esc == '"' || esc == '\\') {
                    out.push_back(esc);
                } else {
                    throw CompileProfileError(
                        "unsupported string escape (only \\\\ and \\\" are recognized)");
                }
            } else if (c == '\n' || c == '\r') {
                throw CompileProfileError("newline inside string literal");
            } else if (static_cast<unsigned char>(c) < 0x20) {
                throw CompileProfileError("unescaped control character in string literal");
            } else {
                out.push_back(c);
            }
        }
    }

    /// Parse an array of quoted strings: `[ "a", "b", ... ]`. Trailing commas
    /// are permitted (matches TOML).
    std::vector<std::string> parse_string_array() {
        if (eof() || raw_[pos_] != '[') {
            throw CompileProfileError("expected array");
        }
        ++pos_;
        std::vector<std::string> out;
        skip_full_ws();
        if (!eof() && raw_[pos_] == ']') {
            ++pos_;
            return out;
        }
        while (true) {
            skip_full_ws();
            if (eof()) {
                throw CompileProfileError("unterminated array");
            }
            out.push_back(parse_string());
            skip_full_ws();
            if (eof()) {
                throw CompileProfileError("unterminated array");
            }
            if (raw_[pos_] == ',') {
                ++pos_;
                skip_full_ws();
                if (!eof() && raw_[pos_] == ']') {
                    ++pos_;
                    return out;
                }
            } else if (raw_[pos_] == ']') {
                ++pos_;
                return out;
            } else {
                throw CompileProfileError("expected ',' or ']' in array");
            }
        }
    }

    /// After consuming a value on a line, ensure the remainder is whitespace
    /// or comment terminated by a newline (or end of input).
    void expect_end_of_line() {
        skip_line_ws();
        if (eof()) {
            return;
        }
        if (raw_[pos_] == '\n' || raw_[pos_] == '\r') {
            ++pos_;
            return;
        }
        std::ostringstream oss;
        oss << "expected end of line after value, found '" << raw_[pos_] << "'";
        throw CompileProfileError(oss.str());
    }

   private:
    std::string_view raw_;
    size_t pos_;
};

// ─── Field container ────────────────────────────────────────────────────────

/// Untyped representation of a single TOML value we care about.
struct TomlValue {
    bool is_string = false;
    bool is_array = false;
    std::string string_value;
    std::vector<std::string> array_value;
};

struct RawProfile {
    std::optional<TomlValue> cxx_standard;
    std::optional<TomlValue> defines;
    std::optional<TomlValue> include_roots;
};

RawProfile parse_toml(std::string_view raw) {
    Tokenizer t(raw);
    RawProfile out;
    while (true) {
        t.skip_full_ws();
        if (t.eof()) {
            break;
        }
        char c = t.peek();
        if (c == '[') {
            throw CompileProfileError("TOML section headers are not permitted here");
        }
        std::string key = t.parse_bare_key();
        t.skip_line_ws();
        if (t.eof() || t.consume() != '=') {
            throw CompileProfileError("expected '=' after key '" + key + "'");
        }
        t.skip_line_ws();
        TomlValue value;
        if (t.eof()) {
            throw CompileProfileError("expected value after '=' for key '" + key + "'");
        }
        char first = t.peek();
        if (first == '"') {
            value.is_string = true;
            value.string_value = t.parse_string();
        } else if (first == '[') {
            value.is_array = true;
            value.array_value = t.parse_string_array();
        } else {
            std::ostringstream oss;
            oss << "unsupported value for key '" << key
                << "' (only quoted strings and string arrays are permitted)";
            throw CompileProfileError(oss.str());
        }
        t.expect_end_of_line();

        std::optional<TomlValue>* slot = nullptr;
        if (key == "cxx_standard") {
            slot = &out.cxx_standard;
        } else if (key == "defines") {
            slot = &out.defines;
        } else if (key == "include_roots") {
            slot = &out.include_roots;
        } else {
            throw CompileProfileError("unknown key: " + key);
        }
        if (slot->has_value()) {
            throw CompileProfileError("duplicate key: " + key);
        }
        *slot = std::move(value);
    }
    return out;
}

// ─── Path validation ────────────────────────────────────────────────────────

/// Return true iff `candidate` is lexically inside `root` (both must already
/// be absolute and canonicalized).
bool is_inside(const fs::path& candidate, const fs::path& root) {
    auto candidate_it = candidate.begin();
    auto root_it = root.begin();
    while (root_it != root.end()) {
        if (candidate_it == candidate.end() || *candidate_it != *root_it) {
            return false;
        }
        ++candidate_it;
        ++root_it;
    }
    return true;
}

fs::path canonicalize_include_root(const fs::path& canonical_repo_root,
                                   const std::string& relative) {
    fs::path rel_path(relative);
    if (rel_path.is_absolute()) {
        throw CompileProfileError("include_roots entry is absolute: " + relative);
    }
    // Reject dotdot components up front so the error message is precise; the
    // canonicalization step below would also fail, but with a less useful
    // filesystem_error.
    for (const auto& seg : rel_path) {
        if (seg == "..") {
            throw CompileProfileError("include_roots entry escapes the repository: " + relative);
        }
    }
    fs::path joined = canonical_repo_root / rel_path;
    std::error_code ec;
    fs::path canonical = fs::canonical(joined, ec);
    if (ec) {
        throw CompileProfileError("include_roots entry could not be resolved: " + relative +
                                  " (" + ec.message() + ")");
    }
    if (!is_inside(canonical, canonical_repo_root)) {
        throw CompileProfileError(
            "include_roots entry resolves outside the repository: " + relative);
    }
    if (!fs::is_directory(canonical, ec)) {
        throw CompileProfileError("include_roots entry is not a directory: " + relative);
    }
    return canonical;
}

}  // namespace

// ─── Public API ─────────────────────────────────────────────────────────────

CompileProfile loadCompileProfile(const fs::path& repositoryRoot) {
    std::error_code ec;
    fs::path canonical_repo = fs::canonical(repositoryRoot, ec);
    if (ec) {
        throw CompileProfileError("repository_root could not be canonicalized: " + ec.message());
    }
    fs::path profile_path =
        canonical_repo / "tools/library-analyzers/cpp/compile-profile.toml";

    std::ifstream f(profile_path);
    if (!f.is_open()) {
        throw CompileProfileError("compile-profile.toml not found under repository_root");
    }
    std::ostringstream oss;
    oss << f.rdbuf();
    RawProfile raw = parse_toml(oss.str());

    if (!raw.cxx_standard.has_value()) {
        throw CompileProfileError("compile-profile.toml is missing 'cxx_standard'");
    }
    if (!raw.defines.has_value()) {
        throw CompileProfileError("compile-profile.toml is missing 'defines'");
    }
    if (!raw.include_roots.has_value()) {
        throw CompileProfileError("compile-profile.toml is missing 'include_roots'");
    }
    if (!raw.cxx_standard->is_string) {
        throw CompileProfileError("'cxx_standard' must be a string");
    }
    if (!raw.defines->is_array) {
        throw CompileProfileError("'defines' must be an array of strings");
    }
    if (!raw.include_roots->is_array) {
        throw CompileProfileError("'include_roots' must be an array of strings");
    }

    CompileProfile out;
    out.repository_root = canonical_repo;
    out.cxx_standard = raw.cxx_standard->string_value;
    out.defines = raw.defines->array_value;
    out.include_roots.reserve(raw.include_roots->array_value.size());
    for (const auto& relative : raw.include_roots->array_value) {
        out.include_roots.push_back(canonicalize_include_root(canonical_repo, relative));
    }
    return out;
}

std::vector<std::string> buildClangArguments(const CompileProfile& profile, const Target& target) {
    // The source file must live under the same repository root the profile
    // was loaded from. `canonical` follows symlinks so a symlinked source
    // file under the repository still passes.
    std::error_code ec;
    fs::path canonical_source = fs::weakly_canonical(target.source_file, ec);
    if (ec) {
        throw CompileProfileError("source_file could not be resolved: " + ec.message());
    }
    if (!canonical_source.is_absolute()) {
        throw CompileProfileError("source_file must be an absolute path");
    }
    if (!is_inside(canonical_source, profile.repository_root)) {
        throw CompileProfileError("source_file is not inside the repository");
    }

    std::vector<std::string> argv;
    argv.reserve(4 + profile.defines.size() + profile.include_roots.size());
    argv.emplace_back("-x");
    argv.emplace_back("c++");
    argv.emplace_back("-std=" + profile.cxx_standard);
    for (const auto& def : profile.defines) {
        argv.emplace_back("-D" + def);
    }
    for (const auto& root : profile.include_roots) {
        argv.emplace_back("-I" + root.string());
    }
    argv.emplace_back(canonical_source.string());
    return argv;
}

}  // namespace ce_cpp
