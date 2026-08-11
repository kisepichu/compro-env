// Handwritten JSON parser/serializer for the ce-cpp adapter (spec §§6.7,
// 6.9; plans 045 & 046).
//
// A vendored `nlohmann::json` would work, but we only need the strict subset
// described by the shared protocol schema. Two hand-written passes stay under
// review-friendly size and avoid importing a 25k-line dependency:
//
//   * `parse_request` / `parse_response` walk a recursive-descent JSON tree
//     value into typed structs, rejecting unknown keys and unrecognized enum
//     tokens.
//   * `serialize_response` emits pretty-printed JSON with alphabetical keys
//     and 2-space indent, matching `serde_json::to_string_pretty` on the
//     Rust side so shared fixtures stay byte-for-byte comparable.

#include "protocol.hpp"

#include <cctype>
#include <cstddef>
#include <cstdio>
#include <memory>
#include <sstream>
#include <string>
#include <utility>
#include <variant>

namespace ce_cpp {

namespace {

// ─── JSON value tree (used only for parsing) ────────────────────────────────

class JsonValue;
using JsonObject = std::vector<std::pair<std::string, std::shared_ptr<JsonValue>>>;
using JsonArray = std::vector<std::shared_ptr<JsonValue>>;

/// Untyped node used only inside parsing helpers. The public API returns
/// typed structs, so the tree does not need to escape this file.
class JsonValue {
   public:
    enum class Kind { Null, Bool, Number, String, Array, Object };

    static std::shared_ptr<JsonValue> null_value() {
        return std::shared_ptr<JsonValue>(new JsonValue(Kind::Null));
    }
    static std::shared_ptr<JsonValue> bool_value(bool b) {
        auto v = std::shared_ptr<JsonValue>(new JsonValue(Kind::Bool));
        v->bool_ = b;
        return v;
    }
    static std::shared_ptr<JsonValue> number_value(uint64_t n) {
        auto v = std::shared_ptr<JsonValue>(new JsonValue(Kind::Number));
        v->number_ = n;
        return v;
    }
    static std::shared_ptr<JsonValue> string_value(std::string s) {
        auto v = std::shared_ptr<JsonValue>(new JsonValue(Kind::String));
        v->string_ = std::move(s);
        return v;
    }
    static std::shared_ptr<JsonValue> array_value(JsonArray a) {
        auto v = std::shared_ptr<JsonValue>(new JsonValue(Kind::Array));
        v->array_ = std::move(a);
        return v;
    }
    static std::shared_ptr<JsonValue> object_value(JsonObject o) {
        auto v = std::shared_ptr<JsonValue>(new JsonValue(Kind::Object));
        v->object_ = std::move(o);
        return v;
    }

    Kind kind() const { return kind_; }
    bool as_bool() const { return bool_; }
    uint64_t as_number() const { return number_; }
    const std::string& as_string() const { return string_; }
    const JsonArray& as_array() const { return array_; }
    const JsonObject& as_object() const { return object_; }

   private:
    explicit JsonValue(Kind k) : kind_(k) {}
    Kind kind_;
    bool bool_ = false;
    uint64_t number_ = 0;
    std::string string_;
    JsonArray array_;
    JsonObject object_;
};

// ─── JSON parser ────────────────────────────────────────────────────────────

class JsonParser {
   public:
    explicit JsonParser(std::string_view raw) : raw_(raw), pos_(0) {}

    std::shared_ptr<JsonValue> parse_document() {
        skip_ws();
        auto v = parse_value();
        skip_ws();
        if (pos_ < raw_.size()) {
            throw ProtocolError("trailing content after top-level value");
        }
        return v;
    }

   private:
    void skip_ws() {
        while (pos_ < raw_.size()) {
            char c = raw_[pos_];
            if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
                ++pos_;
            } else {
                break;
            }
        }
    }

    char consume() {
        if (pos_ >= raw_.size()) {
            throw ProtocolError("unexpected end of input");
        }
        return raw_[pos_++];
    }

    char peek() {
        if (pos_ >= raw_.size()) {
            throw ProtocolError("unexpected end of input");
        }
        return raw_[pos_];
    }

    std::shared_ptr<JsonValue> parse_value() {
        skip_ws();
        char c = peek();
        if (c == '"') return JsonValue::string_value(parse_string_literal());
        if (c == '{') return parse_object();
        if (c == '[') return parse_array();
        if (c == 't' || c == 'f') return parse_bool();
        if (c == 'n') return parse_null();
        if (std::isdigit(static_cast<unsigned char>(c))) {
            return JsonValue::number_value(parse_uint());
        }
        std::ostringstream oss;
        oss << "unexpected character '" << c << "' at offset " << pos_;
        throw ProtocolError(oss.str());
    }

    std::string parse_string_literal() {
        if (consume() != '"') {
            throw ProtocolError("expected quoted string");
        }
        std::string out;
        while (true) {
            if (pos_ >= raw_.size()) {
                throw ProtocolError("unterminated string");
            }
            char c = raw_[pos_++];
            if (c == '"') return out;
            if (c == '\\') {
                if (pos_ >= raw_.size()) {
                    throw ProtocolError("dangling backslash in string");
                }
                char esc = raw_[pos_++];
                switch (esc) {
                    case '"':
                    case '\\':
                    case '/': out.push_back(esc); break;
                    case 'n': out.push_back('\n'); break;
                    case 't': out.push_back('\t'); break;
                    case 'r': out.push_back('\r'); break;
                    case 'b': out.push_back('\b'); break;
                    case 'f': out.push_back('\f'); break;
                    default:
                        throw ProtocolError(
                            "unsupported string escape (only standard JSON escapes are recognized)");
                }
            } else if (static_cast<unsigned char>(c) < 0x20) {
                throw ProtocolError("unescaped control character in string");
            } else {
                out.push_back(c);
            }
        }
    }

    uint64_t parse_uint() {
        std::string digits;
        while (pos_ < raw_.size() && std::isdigit(static_cast<unsigned char>(raw_[pos_]))) {
            digits.push_back(raw_[pos_++]);
        }
        if (digits.empty()) {
            throw ProtocolError("expected an unsigned integer");
        }
        if (digits.size() > 1 && digits[0] == '0') {
            throw ProtocolError("leading zeros are not permitted");
        }
        try {
            return std::stoull(digits);
        } catch (const std::out_of_range&) {
            throw ProtocolError("integer out of range");
        }
    }

    std::shared_ptr<JsonValue> parse_bool() {
        if (raw_.compare(pos_, 4, "true") == 0) {
            pos_ += 4;
            return JsonValue::bool_value(true);
        }
        if (raw_.compare(pos_, 5, "false") == 0) {
            pos_ += 5;
            return JsonValue::bool_value(false);
        }
        throw ProtocolError("expected boolean literal");
    }

    std::shared_ptr<JsonValue> parse_null() {
        if (raw_.compare(pos_, 4, "null") == 0) {
            pos_ += 4;
            return JsonValue::null_value();
        }
        throw ProtocolError("expected null literal");
    }

    std::shared_ptr<JsonValue> parse_array() {
        (void)consume();  // '['
        JsonArray out;
        skip_ws();
        if (pos_ < raw_.size() && raw_[pos_] == ']') {
            ++pos_;
            return JsonValue::array_value(std::move(out));
        }
        while (true) {
            skip_ws();
            out.push_back(parse_value());
            skip_ws();
            if (pos_ >= raw_.size()) {
                throw ProtocolError("unterminated array");
            }
            char c = raw_[pos_++];
            if (c == ',') continue;
            if (c == ']') return JsonValue::array_value(std::move(out));
            throw ProtocolError("expected ',' or ']' in array");
        }
    }

    std::shared_ptr<JsonValue> parse_object() {
        (void)consume();  // '{'
        JsonObject out;
        skip_ws();
        if (pos_ < raw_.size() && raw_[pos_] == '}') {
            ++pos_;
            return JsonValue::object_value(std::move(out));
        }
        while (true) {
            skip_ws();
            std::string key = parse_string_literal();
            skip_ws();
            if (pos_ >= raw_.size() || raw_[pos_++] != ':') {
                throw ProtocolError("expected ':' after object key");
            }
            skip_ws();
            auto value = parse_value();
            out.emplace_back(std::move(key), std::move(value));
            skip_ws();
            if (pos_ >= raw_.size()) {
                throw ProtocolError("unterminated object");
            }
            char c = raw_[pos_++];
            if (c == ',') continue;
            if (c == '}') return JsonValue::object_value(std::move(out));
            throw ProtocolError("expected ',' or '}' in object");
        }
    }

    std::string_view raw_;
    size_t pos_;
};

// ─── Object helpers ─────────────────────────────────────────────────────────

const JsonValue* find(const JsonObject& obj, std::string_view key) {
    for (const auto& [k, v] : obj) {
        if (k == key) return v.get();
    }
    return nullptr;
}

/// Return the value for `key` or throw. Rejects duplicate keys implicitly by
/// only returning the first match — but for strict `deny_unknown_fields`
/// semantics we check duplicates separately in `assert_no_unknown_keys`.
const JsonValue& require(const JsonObject& obj, std::string_view key, std::string_view context) {
    const JsonValue* v = find(obj, key);
    if (v == nullptr) {
        throw ProtocolError(std::string(context) + " is missing required key '" +
                            std::string(key) + "'");
    }
    return *v;
}

void assert_no_unknown_keys(const JsonObject& obj,
                            std::initializer_list<std::string_view> allowed,
                            std::string_view context) {
    // Duplicate-key detection.
    for (size_t i = 0; i < obj.size(); ++i) {
        for (size_t j = i + 1; j < obj.size(); ++j) {
            if (obj[i].first == obj[j].first) {
                throw ProtocolError(std::string(context) + " has duplicate key '" +
                                    obj[i].first + "'");
            }
        }
    }
    for (const auto& [k, _] : obj) {
        bool ok = false;
        for (auto a : allowed) {
            if (k == a) {
                ok = true;
                break;
            }
        }
        if (!ok) {
            throw ProtocolError(std::string(context) + " has unknown key '" + k + "'");
        }
    }
}

const std::string& as_string(const JsonValue& v, std::string_view context) {
    if (v.kind() != JsonValue::Kind::String) {
        throw ProtocolError(std::string(context) + " must be a string");
    }
    return v.as_string();
}

uint32_t as_u32(const JsonValue& v, std::string_view context) {
    if (v.kind() != JsonValue::Kind::Number) {
        throw ProtocolError(std::string(context) + " must be a number");
    }
    uint64_t n = v.as_number();
    if (n > 0xFFFFFFFFULL) {
        throw ProtocolError(std::string(context) + " is out of range for uint32");
    }
    return static_cast<uint32_t>(n);
}

const JsonArray& as_array(const JsonValue& v, std::string_view context) {
    if (v.kind() != JsonValue::Kind::Array) {
        throw ProtocolError(std::string(context) + " must be an array");
    }
    return v.as_array();
}

const JsonObject& as_object(const JsonValue& v, std::string_view context) {
    if (v.kind() != JsonValue::Kind::Object) {
        throw ProtocolError(std::string(context) + " must be an object");
    }
    return v.as_object();
}

// ─── Structured parsers ─────────────────────────────────────────────────────

LibraryTarget parse_library_target(const JsonValue& v) {
    const auto& obj = as_object(v, "libraries entry");
    assert_no_unknown_keys(obj, {"path"}, "libraries entry");
    LibraryTarget out;
    out.path = as_string(require(obj, "path", "libraries entry"), "libraries entry.path");
    return out;
}

SolutionTarget parse_solution_target(const JsonValue& v) {
    const auto& obj = as_object(v, "solutions entry");
    assert_no_unknown_keys(obj, {"id", "root", "entry"}, "solutions entry");
    SolutionTarget out;
    out.id = as_string(require(obj, "id", "solutions entry"), "solutions entry.id");
    out.root = as_string(require(obj, "root", "solutions entry"), "solutions entry.root");
    out.entry = as_string(require(obj, "entry", "solutions entry"), "solutions entry.entry");
    return out;
}

Position parse_position(const JsonValue& v, std::string_view ctx) {
    const auto& obj = as_object(v, ctx);
    assert_no_unknown_keys(obj, {"line", "column"}, ctx);
    Position p;
    p.line = as_u32(require(obj, "line", ctx), std::string(ctx) + ".line");
    if (const JsonValue* col = find(obj, "column"); col != nullptr) {
        if (col->kind() != JsonValue::Kind::Null) {
            p.column = as_u32(*col, std::string(ctx) + ".column");
        }
    }
    return p;
}

Location parse_location(const JsonValue& v, std::string_view ctx) {
    const auto& obj = as_object(v, ctx);
    assert_no_unknown_keys(obj, {"path", "start", "end"}, ctx);
    Location loc;
    loc.path = as_string(require(obj, "path", ctx), std::string(ctx) + ".path");
    loc.start = parse_position(require(obj, "start", ctx), std::string(ctx) + ".start");
    if (const JsonValue* end = find(obj, "end"); end != nullptr) {
        if (end->kind() != JsonValue::Kind::Null) {
            loc.end = parse_position(*end, std::string(ctx) + ".end");
        }
    }
    return loc;
}

std::optional<Location> parse_optional_location(const JsonObject& obj, std::string_view key,
                                                std::string_view ctx) {
    if (const JsonValue* v = find(obj, key); v != nullptr) {
        if (v->kind() == JsonValue::Kind::Null) return std::nullopt;
        return parse_location(*v, std::string(ctx) + "." + std::string(key));
    }
    return std::nullopt;
}

AnalysisState parse_state(const JsonValue& v, std::string_view ctx) {
    const std::string& s = as_string(v, ctx);
    if (s == "complete") return AnalysisState::Complete;
    if (s == "partial") return AnalysisState::Partial;
    if (s == "failed") return AnalysisState::Failed;
    throw ProtocolError(std::string(ctx) + " has unrecognized state '" + s + "'");
}

Dependency parse_dependency(const JsonValue& v) {
    const auto& obj = as_object(v, "dependency");
    const std::string& kind = as_string(require(obj, "kind", "dependency"), "dependency.kind");
    Dependency d;
    if (kind == "internal") {
        assert_no_unknown_keys(obj, {"kind", "location", "path"}, "internal dependency");
        d.kind = DependencyKind::Internal;
        d.path =
            as_string(require(obj, "path", "internal dependency"), "internal dependency.path");
        d.location = parse_optional_location(obj, "location", "internal dependency");
    } else if (kind == "external") {
        assert_no_unknown_keys(obj, {"kind", "location", "name"}, "external dependency");
        d.kind = DependencyKind::External;
        d.name =
            as_string(require(obj, "name", "external dependency"), "external dependency.name");
        d.location = parse_optional_location(obj, "location", "external dependency");
    } else if (kind == "unresolved") {
        assert_no_unknown_keys(obj, {"kind", "location", "key", "display"},
                               "unresolved dependency");
        d.kind = DependencyKind::Unresolved;
        d.key =
            as_string(require(obj, "key", "unresolved dependency"), "unresolved dependency.key");
        d.display = as_string(require(obj, "display", "unresolved dependency"),
                              "unresolved dependency.display");
        d.location = parse_optional_location(obj, "location", "unresolved dependency");
    } else {
        throw ProtocolError("dependency.kind has unrecognized value '" + kind + "'");
    }
    return d;
}

DependencyAnalysis parse_dependency_analysis(const JsonValue& v) {
    const auto& obj = as_object(v, "dependency_analysis");
    assert_no_unknown_keys(obj, {"state", "dependencies"}, "dependency_analysis");
    DependencyAnalysis out;
    out.state =
        parse_state(require(obj, "state", "dependency_analysis"), "dependency_analysis.state");
    if (const JsonValue* deps = find(obj, "dependencies"); deps != nullptr) {
        for (const auto& d : as_array(*deps, "dependency_analysis.dependencies")) {
            out.dependencies.push_back(parse_dependency(*d));
        }
    }
    return out;
}

Symbol parse_symbol(const JsonValue& v) {
    const auto& obj = as_object(v, "symbol");
    assert_no_unknown_keys(
        obj,
        {"name", "kind", "qualified_name", "search_names", "signature", "location"},
        "symbol");
    Symbol s;
    s.name = as_string(require(obj, "name", "symbol"), "symbol.name");
    s.kind = as_string(require(obj, "kind", "symbol"), "symbol.kind");
    if (const JsonValue* q = find(obj, "qualified_name"); q != nullptr) {
        if (q->kind() != JsonValue::Kind::Null) {
            s.qualified_name = as_string(*q, "symbol.qualified_name");
        }
    }
    if (const JsonValue* sn = find(obj, "search_names"); sn != nullptr) {
        for (const auto& e : as_array(*sn, "symbol.search_names")) {
            std::string alias = as_string(*e, "symbol.search_names[]");
            if (alias.empty()) {
                throw ProtocolError("symbol.search_names[] must be non-empty");
            }
            s.search_names.push_back(std::move(alias));
        }
    }
    if (const JsonValue* sig = find(obj, "signature"); sig != nullptr) {
        if (sig->kind() != JsonValue::Kind::Null) {
            s.signature = as_string(*sig, "symbol.signature");
        }
    }
    s.location = parse_optional_location(obj, "location", "symbol");

    // spec §6.3: when `search_names` is present, it must contain `name`
    // (and `qualified_name` when the symbol has one). We reject inputs
    // that omit them so an out-of-date adapter can't slip past validation
    // and land a symbol the core cannot exact-match.
    if (!s.search_names.empty()) {
        auto contains = [&](const std::string& v) {
            for (const auto& a : s.search_names) {
                if (a == v) return true;
            }
            return false;
        };
        if (!contains(s.name)) {
            throw ProtocolError("symbol.search_names must include symbol.name");
        }
        if (s.qualified_name.has_value() && !contains(*s.qualified_name)) {
            throw ProtocolError(
                "symbol.search_names must include symbol.qualified_name when set");
        }
    }
    return s;
}

SymbolAnalysis parse_symbol_analysis(const JsonValue& v) {
    const auto& obj = as_object(v, "symbol_analysis");
    assert_no_unknown_keys(obj, {"state", "symbols"}, "symbol_analysis");
    SymbolAnalysis out;
    out.state = parse_state(require(obj, "state", "symbol_analysis"), "symbol_analysis.state");
    if (const JsonValue* syms = find(obj, "symbols"); syms != nullptr) {
        for (const auto& s : as_array(*syms, "symbol_analysis.symbols")) {
            out.symbols.push_back(parse_symbol(*s));
        }
    }
    return out;
}

Diagnostic parse_diagnostic(const JsonValue& v) {
    const auto& obj = as_object(v, "diagnostic");
    assert_no_unknown_keys(obj, {"severity", "code", "message", "location"}, "diagnostic");
    Diagnostic d;
    const std::string& severity =
        as_string(require(obj, "severity", "diagnostic"), "diagnostic.severity");
    if (severity == "info") d.severity = Severity::Info;
    else if (severity == "warning") d.severity = Severity::Warning;
    else if (severity == "error") d.severity = Severity::Error;
    else throw ProtocolError("diagnostic.severity has unrecognized value '" + severity + "'");
    d.code = as_string(require(obj, "code", "diagnostic"), "diagnostic.code");
    d.message = as_string(require(obj, "message", "diagnostic"), "diagnostic.message");
    d.location = parse_optional_location(obj, "location", "diagnostic");
    return d;
}

LibraryAnalysis parse_library_analysis(const JsonValue& v) {
    const auto& obj = as_object(v, "library analysis");
    assert_no_unknown_keys(obj,
                           {"path", "dependency_analysis", "symbol_analysis", "diagnostics"},
                           "library analysis");
    LibraryAnalysis out;
    out.path = as_string(require(obj, "path", "library analysis"), "library analysis.path");
    out.dependency_analysis =
        parse_dependency_analysis(require(obj, "dependency_analysis", "library analysis"));
    out.symbol_analysis =
        parse_symbol_analysis(require(obj, "symbol_analysis", "library analysis"));
    if (const JsonValue* diags = find(obj, "diagnostics"); diags != nullptr) {
        for (const auto& d : as_array(*diags, "library analysis.diagnostics")) {
            out.diagnostics.push_back(parse_diagnostic(*d));
        }
    }
    return out;
}

SolutionAnalysis parse_solution_analysis(const JsonValue& v) {
    const auto& obj = as_object(v, "solution analysis");
    assert_no_unknown_keys(obj, {"id", "dependency_analysis", "diagnostics"},
                           "solution analysis");
    SolutionAnalysis out;
    out.id = as_string(require(obj, "id", "solution analysis"), "solution analysis.id");
    out.dependency_analysis =
        parse_dependency_analysis(require(obj, "dependency_analysis", "solution analysis"));
    if (const JsonValue* diags = find(obj, "diagnostics"); diags != nullptr) {
        for (const auto& d : as_array(*diags, "solution analysis.diagnostics")) {
            out.diagnostics.push_back(parse_diagnostic(*d));
        }
    }
    return out;
}

ToolchainIdentity parse_toolchain(const JsonValue& v) {
    const auto& obj = as_object(v, "toolchain");
    assert_no_unknown_keys(obj, {"name", "version", "target"}, "toolchain");
    ToolchainIdentity t;
    t.name = as_string(require(obj, "name", "toolchain"), "toolchain.name");
    t.version = as_string(require(obj, "version", "toolchain"), "toolchain.version");
    if (const JsonValue* target = find(obj, "target"); target != nullptr) {
        if (target->kind() != JsonValue::Kind::Null) {
            t.target = as_string(*target, "toolchain.target");
        }
    }
    return t;
}

// ─── Serializer ─────────────────────────────────────────────────────────────

/// Pretty-printing JSON writer. Follows `serde_json::to_string_pretty`
/// conventions: 2-space indent, newline after `{`, `[`, and commas, closing
/// brace/bracket back at the parent indent level. Empty objects and arrays
/// stay on one line (`{}` / `[]`).
///
/// Scope tracking (`first_field_stack_`) is a stack so nested objects/arrays
/// each keep their own "is this the first element" state — otherwise a
/// closing bracket would clobber the parent scope's flag.
class Writer {
   public:
    Writer() = default;

    const std::string& str() const { return out_; }

    void write_json_string(std::string_view value) {
        out_.push_back('"');
        for (char c : value) {
            switch (c) {
                case '"': out_.append("\\\""); break;
                case '\\': out_.append("\\\\"); break;
                case '\n': out_.append("\\n"); break;
                case '\r': out_.append("\\r"); break;
                case '\t': out_.append("\\t"); break;
                case '\b': out_.append("\\b"); break;
                case '\f': out_.append("\\f"); break;
                default:
                    if (static_cast<unsigned char>(c) < 0x20) {
                        char buf[8];
                        std::snprintf(buf, sizeof(buf), "\\u%04x",
                                      static_cast<unsigned char>(c));
                        out_.append(buf);
                    } else {
                        out_.push_back(c);
                    }
            }
        }
        out_.push_back('"');
    }

    void write_uint(uint64_t n) { out_.append(std::to_string(n)); }

    void begin_object() {
        out_.push_back('{');
        first_field_stack_.push_back(1);
    }

    void end_object(bool was_empty) {
        first_field_stack_.pop_back();
        if (was_empty) {
            out_.push_back('}');
        } else {
            out_.push_back('\n');
            write_indent();
            out_.push_back('}');
        }
    }

    void begin_array() {
        out_.push_back('[');
        first_field_stack_.push_back(1);
    }

    void end_array(bool was_empty) {
        first_field_stack_.pop_back();
        if (was_empty) {
            out_.push_back(']');
        } else {
            out_.push_back('\n');
            write_indent();
            out_.push_back(']');
        }
    }

    /// Write the leading `,\n<indent>` for an object field except the first
    /// field which gets only `\n<indent>`. Same helper is used for array
    /// elements.
    void field_prefix() {
        auto& first = first_field_stack_.back();
        if (first) {
            first = 0;
        } else {
            out_.push_back(',');
        }
        out_.push_back('\n');
        write_indent();
    }

    /// Alias for `field_prefix` at array-element sites.
    void element_prefix() { field_prefix(); }

    void write_field_name(std::string_view name) {
        write_json_string(name);
        out_.append(": ");
    }

   private:
    void write_indent() {
        // Depth = current stack size (0 at top-level).
        for (size_t i = 0; i < first_field_stack_.size(); ++i) {
            out_.append("  ");
        }
    }

    std::string out_;
    std::vector<uint8_t> first_field_stack_;
};

void write_position(Writer& w, const Position& p) {
    w.begin_object();
    if (p.column.has_value()) {
        w.field_prefix();
        w.write_field_name("column");
        w.write_uint(*p.column);
    }
    w.field_prefix();
    w.write_field_name("line");
    w.write_uint(p.line);
    w.end_object(false);
}

void write_location(Writer& w, const Location& loc) {
    w.begin_object();
    // Alphabetical: end, path, start.
    if (loc.end.has_value()) {
        w.field_prefix();
        w.write_field_name("end");
        write_position(w, *loc.end);
    }
    w.field_prefix();
    w.write_field_name("path");
    w.write_json_string(loc.path);
    w.field_prefix();
    w.write_field_name("start");
    write_position(w, loc.start);
    w.end_object(false);
}

void write_dependency(Writer& w, const Dependency& d) {
    w.begin_object();
    switch (d.kind) {
        case DependencyKind::Internal:
            w.field_prefix();
            w.write_field_name("kind");
            w.write_json_string("internal");
            if (d.location.has_value()) {
                w.field_prefix();
                w.write_field_name("location");
                write_location(w, *d.location);
            }
            w.field_prefix();
            w.write_field_name("path");
            w.write_json_string(d.path);
            break;
        case DependencyKind::External:
            w.field_prefix();
            w.write_field_name("kind");
            w.write_json_string("external");
            if (d.location.has_value()) {
                w.field_prefix();
                w.write_field_name("location");
                write_location(w, *d.location);
            }
            w.field_prefix();
            w.write_field_name("name");
            w.write_json_string(d.name);
            break;
        case DependencyKind::Unresolved:
            w.field_prefix();
            w.write_field_name("display");
            w.write_json_string(d.display);
            w.field_prefix();
            w.write_field_name("key");
            w.write_json_string(d.key);
            w.field_prefix();
            w.write_field_name("kind");
            w.write_json_string("unresolved");
            if (d.location.has_value()) {
                w.field_prefix();
                w.write_field_name("location");
                write_location(w, *d.location);
            }
            break;
    }
    w.end_object(false);
}

std::string state_to_str(AnalysisState s) {
    switch (s) {
        case AnalysisState::Complete: return "complete";
        case AnalysisState::Partial: return "partial";
        case AnalysisState::Failed: return "failed";
    }
    return "complete";
}

std::string severity_to_str(Severity s) {
    switch (s) {
        case Severity::Info: return "info";
        case Severity::Warning: return "warning";
        case Severity::Error: return "error";
    }
    return "error";
}

void write_dependency_analysis(Writer& w, const DependencyAnalysis& da) {
    w.begin_object();
    w.field_prefix();
    w.write_field_name("dependencies");
    if (da.dependencies.empty()) {
        w.begin_array();
        w.end_array(true);
    } else {
        w.begin_array();
        for (const auto& d : da.dependencies) {
            w.element_prefix();
            write_dependency(w, d);
        }
        w.end_array(false);
    }
    w.field_prefix();
    w.write_field_name("state");
    w.write_json_string(state_to_str(da.state));
    w.end_object(false);
}

void write_symbol(Writer& w, const Symbol& s) {
    w.begin_object();
    // Alphabetical: kind, location, name, qualified_name, search_names, signature.
    w.field_prefix();
    w.write_field_name("kind");
    w.write_json_string(s.kind);
    if (s.location.has_value()) {
        w.field_prefix();
        w.write_field_name("location");
        write_location(w, *s.location);
    }
    w.field_prefix();
    w.write_field_name("name");
    w.write_json_string(s.name);
    if (s.qualified_name.has_value()) {
        w.field_prefix();
        w.write_field_name("qualified_name");
        w.write_json_string(*s.qualified_name);
    }
    if (!s.search_names.empty()) {
        w.field_prefix();
        w.write_field_name("search_names");
        w.begin_array();
        for (const auto& n : s.search_names) {
            w.element_prefix();
            w.write_json_string(n);
        }
        w.end_array(false);
    }
    if (s.signature.has_value()) {
        w.field_prefix();
        w.write_field_name("signature");
        w.write_json_string(*s.signature);
    }
    w.end_object(false);
}

void write_symbol_analysis(Writer& w, const SymbolAnalysis& sa) {
    w.begin_object();
    w.field_prefix();
    w.write_field_name("state");
    w.write_json_string(state_to_str(sa.state));
    w.field_prefix();
    w.write_field_name("symbols");
    if (sa.symbols.empty()) {
        w.begin_array();
        w.end_array(true);
    } else {
        w.begin_array();
        for (const auto& s : sa.symbols) {
            w.element_prefix();
            write_symbol(w, s);
        }
        w.end_array(false);
    }
    w.end_object(false);
}

void write_diagnostic(Writer& w, const Diagnostic& d) {
    w.begin_object();
    w.field_prefix();
    w.write_field_name("code");
    w.write_json_string(d.code);
    if (d.location.has_value()) {
        w.field_prefix();
        w.write_field_name("location");
        write_location(w, *d.location);
    }
    w.field_prefix();
    w.write_field_name("message");
    w.write_json_string(d.message);
    w.field_prefix();
    w.write_field_name("severity");
    w.write_json_string(severity_to_str(d.severity));
    w.end_object(false);
}

void write_diagnostic_array(Writer& w, const std::vector<Diagnostic>& diags) {
    if (diags.empty()) {
        w.begin_array();
        w.end_array(true);
    } else {
        w.begin_array();
        for (const auto& d : diags) {
            w.element_prefix();
            write_diagnostic(w, d);
        }
        w.end_array(false);
    }
}

void write_library_analysis(Writer& w, const LibraryAnalysis& la) {
    w.begin_object();
    w.field_prefix();
    w.write_field_name("dependency_analysis");
    write_dependency_analysis(w, la.dependency_analysis);
    w.field_prefix();
    w.write_field_name("diagnostics");
    write_diagnostic_array(w, la.diagnostics);
    w.field_prefix();
    w.write_field_name("path");
    w.write_json_string(la.path);
    w.field_prefix();
    w.write_field_name("symbol_analysis");
    write_symbol_analysis(w, la.symbol_analysis);
    w.end_object(false);
}

void write_solution_analysis(Writer& w, const SolutionAnalysis& sa) {
    w.begin_object();
    w.field_prefix();
    w.write_field_name("dependency_analysis");
    write_dependency_analysis(w, sa.dependency_analysis);
    w.field_prefix();
    w.write_field_name("diagnostics");
    write_diagnostic_array(w, sa.diagnostics);
    w.field_prefix();
    w.write_field_name("id");
    w.write_json_string(sa.id);
    w.end_object(false);
}

void write_toolchain(Writer& w, const ToolchainIdentity& t) {
    w.begin_object();
    w.field_prefix();
    w.write_field_name("name");
    w.write_json_string(t.name);
    if (t.target.has_value()) {
        w.field_prefix();
        w.write_field_name("target");
        w.write_json_string(*t.target);
    }
    w.field_prefix();
    w.write_field_name("version");
    w.write_json_string(t.version);
    w.end_object(false);
}

void write_adapter(Writer& w, const AdapterIdentity& a) {
    w.begin_object();
    w.field_prefix();
    w.write_field_name("name");
    w.write_json_string(a.name);
    w.field_prefix();
    w.write_field_name("toolchains");
    if (a.toolchains.empty()) {
        w.begin_array();
        w.end_array(true);
    } else {
        w.begin_array();
        for (const auto& t : a.toolchains) {
            w.element_prefix();
            write_toolchain(w, t);
        }
        w.end_array(false);
    }
    w.field_prefix();
    w.write_field_name("version");
    w.write_json_string(a.version);
    w.end_object(false);
}

}  // namespace

// ─── parse_request ──────────────────────────────────────────────────────────

AnalysisRequest parse_request(std::string_view raw) {
    JsonParser p(raw);
    auto root = p.parse_document();
    const auto& obj = as_object(*root, "AnalysisRequest");
    assert_no_unknown_keys(
        obj,
        {"schema_version", "repository_root", "language", "libraries", "solutions"},
        "AnalysisRequest");
    AnalysisRequest req;
    req.schema_version =
        as_u32(require(obj, "schema_version", "AnalysisRequest"), "AnalysisRequest.schema_version");
    if (req.schema_version != SCHEMA_VERSION) {
        throw ProtocolError("unsupported schema_version");
    }
    req.repository_root = as_string(require(obj, "repository_root", "AnalysisRequest"),
                                    "AnalysisRequest.repository_root");
    req.language =
        as_string(require(obj, "language", "AnalysisRequest"), "AnalysisRequest.language");
    for (const auto& lib :
         as_array(require(obj, "libraries", "AnalysisRequest"), "AnalysisRequest.libraries")) {
        req.libraries.push_back(parse_library_target(*lib));
    }
    for (const auto& sol :
         as_array(require(obj, "solutions", "AnalysisRequest"), "AnalysisRequest.solutions")) {
        req.solutions.push_back(parse_solution_target(*sol));
    }
    return req;
}

// ─── serialize_response ─────────────────────────────────────────────────────

std::string serialize_response(const AnalysisResponse& response) {
    Writer w;
    w.begin_object();
    w.field_prefix();
    w.write_field_name("adapter");
    write_adapter(w, response.adapter);

    w.field_prefix();
    w.write_field_name("libraries");
    if (response.libraries.empty()) {
        w.begin_array();
        w.end_array(true);
    } else {
        w.begin_array();
        for (const auto& la : response.libraries) {
            w.element_prefix();
            write_library_analysis(w, la);
        }
        w.end_array(false);
    }

    w.field_prefix();
    w.write_field_name("schema_version");
    w.write_uint(response.schema_version);

    w.field_prefix();
    w.write_field_name("solutions");
    if (response.solutions.empty()) {
        w.begin_array();
        w.end_array(true);
    } else {
        w.begin_array();
        for (const auto& sa : response.solutions) {
            w.element_prefix();
            write_solution_analysis(w, sa);
        }
        w.end_array(false);
    }
    w.end_object(false);
    return w.str();
}

// ─── parse_response ─────────────────────────────────────────────────────────

AnalysisResponse parse_response(std::string_view raw) {
    JsonParser p(raw);
    auto root = p.parse_document();
    const auto& obj = as_object(*root, "AnalysisResponse");
    assert_no_unknown_keys(obj,
                           {"schema_version", "adapter", "libraries", "solutions"},
                           "AnalysisResponse");
    AnalysisResponse resp;
    resp.schema_version =
        as_u32(require(obj, "schema_version", "AnalysisResponse"),
               "AnalysisResponse.schema_version");
    if (resp.schema_version != SCHEMA_VERSION) {
        throw ProtocolError("unsupported schema_version");
    }
    const auto& adapter = as_object(require(obj, "adapter", "AnalysisResponse"),
                                    "AnalysisResponse.adapter");
    assert_no_unknown_keys(adapter, {"name", "version", "toolchains"}, "adapter");
    resp.adapter.name = as_string(require(adapter, "name", "adapter"), "adapter.name");
    resp.adapter.version = as_string(require(adapter, "version", "adapter"), "adapter.version");
    for (const auto& t :
         as_array(require(adapter, "toolchains", "adapter"), "adapter.toolchains")) {
        resp.adapter.toolchains.push_back(parse_toolchain(*t));
    }
    for (const auto& la :
         as_array(require(obj, "libraries", "AnalysisResponse"), "AnalysisResponse.libraries")) {
        resp.libraries.push_back(parse_library_analysis(*la));
    }
    for (const auto& sa :
         as_array(require(obj, "solutions", "AnalysisResponse"), "AnalysisResponse.solutions")) {
        resp.solutions.push_back(parse_solution_analysis(*sa));
    }
    return resp;
}

}  // namespace ce_cpp
