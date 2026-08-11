// Handwritten JSON parser/serializer for the ce-cpp adapter (spec §§6.7, 6.9;
// plan 045 Task 2).
//
// A vendored `nlohmann::json` would work, but for the empty handshake we
// only need to read a handful of fixed fields and write a fixed shape. A tiny
// strict recursive-descent parser keeps the source under review-friendly size
// and avoids importing a 25k-line dependency for one JSON document.
//
// The parser supports UTF-8 input, arbitrary whitespace, quoted strings with
// `\"` and `\\` escapes (all other backslash escapes are rejected — the
// fixtures never need them), unsigned integers, `null`, empty arrays `[]`,
// empty objects `{}`, and simple key/value objects with `deny_unknown_fields`
// semantics. Anything else raises `ProtocolError`.

#include "protocol.hpp"

#include <cctype>
#include <sstream>
#include <string>

namespace ce_cpp {

namespace {

// ─── Parser ─────────────────────────────────────────────────────────────────

class Parser {
   public:
    explicit Parser(std::string_view raw) : raw_(raw), pos_(0) {}

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

    bool eof() const { return pos_ >= raw_.size(); }

    char peek() {
        if (eof()) {
            throw ProtocolError("unexpected end of input");
        }
        return raw_[pos_];
    }

    char consume() {
        if (eof()) {
            throw ProtocolError("unexpected end of input");
        }
        return raw_[pos_++];
    }

    void expect(char c) {
        skip_ws();
        char got = consume();
        if (got != c) {
            std::ostringstream oss;
            oss << "expected '" << c << "', got '" << got << "'";
            throw ProtocolError(oss.str());
        }
    }

    /// Parse a quoted JSON string. Only `\"` and `\\` escapes are supported;
    /// any other backslash sequence is rejected.
    std::string parse_string() {
        skip_ws();
        expect_no_ws('"');
        std::string out;
        while (true) {
            if (eof()) {
                throw ProtocolError("unterminated string");
            }
            char c = raw_[pos_++];
            if (c == '"') {
                return out;
            }
            if (c == '\\') {
                if (eof()) {
                    throw ProtocolError("dangling backslash in string");
                }
                char esc = raw_[pos_++];
                if (esc == '"' || esc == '\\') {
                    out.push_back(esc);
                } else {
                    throw ProtocolError(
                        "unsupported string escape (only \\\\ and \\\" are recognized)");
                }
            } else {
                // JSON forbids unescaped control characters (U+0000..U+001F)
                // in string literals; reject them so the strict adapter never
                // silently swallows malformed input.
                if (static_cast<unsigned char>(c) < 0x20) {
                    throw ProtocolError(
                        "unescaped control character in string");
                }
                out.push_back(c);
            }
        }
    }

    uint32_t parse_uint32() {
        skip_ws();
        std::string digits;
        while (!eof() && std::isdigit(static_cast<unsigned char>(raw_[pos_]))) {
            digits.push_back(raw_[pos_++]);
        }
        if (digits.empty()) {
            throw ProtocolError("expected an unsigned integer");
        }
        // Reject leading zeros (except the literal "0") to match strict JSON.
        if (digits.size() > 1 && digits[0] == '0') {
            throw ProtocolError("leading zeros are not permitted");
        }
        // Fit-in-uint32 check via std::stoull.
        try {
            unsigned long long v = std::stoull(digits);
            if (v > 0xFFFFFFFFULL) {
                throw ProtocolError("integer out of range for uint32");
            }
            return static_cast<uint32_t>(v);
        } catch (const std::out_of_range&) {
            throw ProtocolError("integer out of range for uint32");
        }
    }

    /// Consume an empty array `[]`. Any element is a protocol error at this
    /// stage of the handshake. Whitespace inside the brackets is allowed.
    void parse_empty_array() {
        skip_ws();
        expect_no_ws('[');
        skip_ws();
        if (!eof() && raw_[pos_] == ']') {
            ++pos_;
            return;
        }
        throw ProtocolError(
            "empty handshake requires an empty array; adapter received "
            "non-empty payload");
    }

    /// Skip the value at the current position when we do not care about it.
    /// Used only for future-compat inside strict wrappers, but the empty
    /// handshake never invokes it. Kept private and unused for now.

    /// Ensures the next non-whitespace character is `c`. Unlike `expect`, this
    /// helper never re-skips whitespace after the char, which matters when the
    /// caller is scanning character-by-character.
    void expect_no_ws(char c) {
        char got = consume();
        if (got != c) {
            std::ostringstream oss;
            oss << "expected '" << c << "', got '" << got << "'";
            throw ProtocolError(oss.str());
        }
    }

    /// After we finish parsing the top-level value, trailing whitespace is OK
    /// but any other content is a protocol error.
    void expect_trailing_ws_only() {
        skip_ws();
        if (!eof()) {
            throw ProtocolError("trailing content after top-level value");
        }
    }

   private:
    std::string_view raw_;
    size_t pos_;
};

// ─── Serializer ─────────────────────────────────────────────────────────────

/// Write a JSON string literal. Only `"` and `\` need escaping in our field
/// set; other control characters would already be rejected upstream, but we
/// still escape `"` and `\` defensively.
void write_json_string(std::string& out, std::string_view value) {
    out.push_back('"');
    for (char c : value) {
        if (c == '"' || c == '\\') {
            out.push_back('\\');
            out.push_back(c);
        } else {
            out.push_back(c);
        }
    }
    out.push_back('"');
}

void write_toolchain(std::string& out, const ToolchainIdentity& t) {
    out.push_back('{');
    out.append("\"name\":");
    write_json_string(out, t.name);
    out.append(",\"version\":");
    write_json_string(out, t.version);
    if (t.target.has_value()) {
        out.append(",\"target\":");
        write_json_string(out, *t.target);
    }
    out.push_back('}');
}

}  // namespace

// ─── parse_request ──────────────────────────────────────────────────────────

AnalysisRequest parse_request(std::string_view raw) {
    Parser p(raw);
    p.skip_ws();
    p.expect('{');
    AnalysisRequest req;
    bool have_schema = false;
    bool have_repo = false;
    bool have_language = false;
    bool have_libraries = false;
    bool have_solutions = false;
    bool first = true;
    while (true) {
        p.skip_ws();
        if (p.eof()) {
            throw ProtocolError("unterminated top-level object");
        }
        if (p.peek() == '}') {
            (void)p.consume();
            break;
        }
        if (!first) {
            p.expect(',');
            p.skip_ws();
        }
        first = false;
        std::string key = p.parse_string();
        p.skip_ws();
        p.expect(':');
        if (key == "schema_version") {
            if (have_schema) {
                throw ProtocolError("duplicate key: schema_version");
            }
            req.schema_version = p.parse_uint32();
            have_schema = true;
        } else if (key == "repository_root") {
            if (have_repo) {
                throw ProtocolError("duplicate key: repository_root");
            }
            req.repository_root = p.parse_string();
            have_repo = true;
        } else if (key == "language") {
            if (have_language) {
                throw ProtocolError("duplicate key: language");
            }
            req.language = p.parse_string();
            have_language = true;
        } else if (key == "libraries") {
            if (have_libraries) {
                throw ProtocolError("duplicate key: libraries");
            }
            p.parse_empty_array();
            have_libraries = true;
        } else if (key == "solutions") {
            if (have_solutions) {
                throw ProtocolError("duplicate key: solutions");
            }
            p.parse_empty_array();
            have_solutions = true;
        } else {
            throw ProtocolError(std::string("unknown request key: ") + key);
        }
    }
    p.expect_trailing_ws_only();

    if (!have_schema) {
        throw ProtocolError("request is missing schema_version");
    }
    if (req.schema_version != SCHEMA_VERSION) {
        throw ProtocolError("unsupported schema_version");
    }
    if (!have_repo) {
        throw ProtocolError("request is missing repository_root");
    }
    if (!have_language) {
        throw ProtocolError("request is missing language");
    }
    if (!have_libraries) {
        throw ProtocolError("request is missing libraries");
    }
    if (!have_solutions) {
        throw ProtocolError("request is missing solutions");
    }
    return req;
}

// ─── serialize_response ─────────────────────────────────────────────────────

std::string serialize_response(const AnalysisResponse& response) {
    std::string out;
    out.push_back('{');
    out.append("\"schema_version\":");
    out.append(std::to_string(response.schema_version));
    out.append(",\"adapter\":{");
    out.append("\"name\":");
    write_json_string(out, response.adapter.name);
    out.append(",\"version\":");
    write_json_string(out, response.adapter.version);
    out.append(",\"toolchains\":[");
    for (size_t i = 0; i < response.adapter.toolchains.size(); ++i) {
        if (i > 0) {
            out.push_back(',');
        }
        write_toolchain(out, response.adapter.toolchains[i]);
    }
    out.append("]}");  // close toolchains + adapter
    out.append(",\"libraries\":[]");
    out.append(",\"solutions\":[]");
    out.push_back('}');
    return out;
}

// ─── parse_response (tests only) ────────────────────────────────────────────

AnalysisResponse parse_response(std::string_view raw) {
    Parser p(raw);
    p.skip_ws();
    p.expect('{');
    AnalysisResponse resp;
    bool have_schema = false;
    bool have_adapter = false;
    bool have_libraries = false;
    bool have_solutions = false;
    bool first = true;
    while (true) {
        p.skip_ws();
        if (p.eof()) {
            throw ProtocolError("unterminated top-level object");
        }
        if (p.peek() == '}') {
            (void)p.consume();
            break;
        }
        if (!first) {
            p.expect(',');
            p.skip_ws();
        }
        first = false;
        std::string key = p.parse_string();
        p.skip_ws();
        p.expect(':');
        if (key == "schema_version") {
            resp.schema_version = p.parse_uint32();
            have_schema = true;
        } else if (key == "adapter") {
            p.skip_ws();
            p.expect('{');
            AdapterIdentity a;
            bool sub_first = true;
            bool have_name = false;
            bool have_version = false;
            bool have_toolchains = false;
            while (true) {
                p.skip_ws();
                if (p.eof()) {
                    throw ProtocolError("unterminated adapter object");
                }
                if (p.peek() == '}') {
                    (void)p.consume();
                    break;
                }
                if (!sub_first) {
                    p.expect(',');
                    p.skip_ws();
                }
                sub_first = false;
                std::string subkey = p.parse_string();
                p.skip_ws();
                p.expect(':');
                if (subkey == "name") {
                    a.name = p.parse_string();
                    have_name = true;
                } else if (subkey == "version") {
                    a.version = p.parse_string();
                    have_version = true;
                } else if (subkey == "toolchains") {
                    p.skip_ws();
                    p.expect('[');
                    bool arr_first = true;
                    while (true) {
                        p.skip_ws();
                        if (p.eof()) {
                            throw ProtocolError("unterminated toolchains array");
                        }
                        if (p.peek() == ']') {
                            (void)p.consume();
                            break;
                        }
                        if (!arr_first) {
                            p.expect(',');
                            p.skip_ws();
                        }
                        arr_first = false;
                        p.expect('{');
                        ToolchainIdentity t;
                        bool tc_first = true;
                        bool tc_have_name = false;
                        bool tc_have_version = false;
                        while (true) {
                            p.skip_ws();
                            if (p.eof()) {
                                throw ProtocolError("unterminated toolchain object");
                            }
                            if (p.peek() == '}') {
                                (void)p.consume();
                                break;
                            }
                            if (!tc_first) {
                                p.expect(',');
                                p.skip_ws();
                            }
                            tc_first = false;
                            std::string tk = p.parse_string();
                            p.skip_ws();
                            p.expect(':');
                            if (tk == "name") {
                                t.name = p.parse_string();
                                tc_have_name = true;
                            } else if (tk == "version") {
                                t.version = p.parse_string();
                                tc_have_version = true;
                            } else if (tk == "target") {
                                t.target = p.parse_string();
                            } else {
                                throw ProtocolError("unknown toolchain key: " + tk);
                            }
                        }
                        if (!tc_have_name || !tc_have_version) {
                            throw ProtocolError("toolchain requires name and version");
                        }
                        a.toolchains.push_back(std::move(t));
                    }
                    have_toolchains = true;
                } else {
                    throw ProtocolError("unknown adapter key: " + subkey);
                }
            }
            if (!have_name || !have_version || !have_toolchains) {
                throw ProtocolError("adapter requires name, version, toolchains");
            }
            resp.adapter = std::move(a);
            have_adapter = true;
        } else if (key == "libraries") {
            p.parse_empty_array();
            have_libraries = true;
        } else if (key == "solutions") {
            p.parse_empty_array();
            have_solutions = true;
        } else {
            throw ProtocolError("unknown response key: " + key);
        }
    }
    p.expect_trailing_ws_only();
    if (!have_schema || !have_adapter || !have_libraries || !have_solutions) {
        throw ProtocolError("response is missing a required key");
    }
    if (resp.schema_version != SCHEMA_VERSION) {
        throw ProtocolError("unsupported schema_version");
    }
    return resp;
}

}  // namespace ce_cpp
