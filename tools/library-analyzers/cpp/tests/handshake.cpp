// C++ unit tests for the ce-cpp protocol layer (spec §§6.7, 6.9; plan 045
// Task 2).
//
// These tests do not spawn the compiled `cpp-analyzer` binary; they run the
// same parse/serialize code the executable uses, exercised against the shared
// `tools/library-analyzers/protocol/fixtures/` documents. `CE_CPP_FIXTURE_DIR`
// is passed in by CMake so the test binary can locate the fixtures from any
// build directory.

#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>

#include "protocol.hpp"

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

#define CE_EXPECT_EQ(a, b)                                                                      \
    do {                                                                                        \
        auto _a = (a);                                                                          \
        auto _b = (b);                                                                          \
        if (!(_a == _b)) {                                                                      \
            std::cerr << __FILE__ << ":" << __LINE__ << ": expected equality: " << #a << " == " \
                      << #b << std::endl;                                                       \
            ++failures;                                                                         \
        }                                                                                       \
    } while (0)

#define CE_EXPECT_THROWS(expr, exc_type)                                                       \
    do {                                                                                       \
        bool _threw = false;                                                                   \
        try {                                                                                  \
            (void)(expr);                                                                      \
        } catch (const exc_type&) {                                                            \
            _threw = true;                                                                     \
        } catch (...) {                                                                        \
            std::cerr << __FILE__ << ":" << __LINE__ << ": wrong exception type" << std::endl; \
            ++failures;                                                                        \
        }                                                                                      \
        if (!_threw) {                                                                         \
            std::cerr << __FILE__ << ":" << __LINE__ << ": expected " << #exc_type             \
                      << " from " << #expr << std::endl;                                       \
            ++failures;                                                                        \
        }                                                                                      \
    } while (0)

std::string read_file(const std::string& path) {
    std::ifstream f(path);
    if (!f.is_open()) {
        std::cerr << "failed to open fixture: " << path << std::endl;
        std::abort();
    }
    std::ostringstream ss;
    ss << f.rdbuf();
    return ss.str();
}

void test_parse_empty_request_fixture() {
    const std::string raw = read_file(std::string(CE_CPP_FIXTURE_DIR) + "/empty-request.json");
    auto req = ce_cpp::parse_request(raw);
    CE_EXPECT_EQ(req.schema_version, ce_cpp::SCHEMA_VERSION);
    CE_EXPECT_EQ(req.repository_root, std::string("."));
    CE_EXPECT_EQ(req.language, std::string("rust"));  // fixture is language-agnostic
}

void test_reject_unknown_key() {
    const std::string bad = R"({
        "schema_version": 1,
        "repository_root": ".",
        "language": "cpp",
        "libraries": [],
        "solutions": [],
        "extra_key": "nope"
    })";
    CE_EXPECT_THROWS(ce_cpp::parse_request(bad), ce_cpp::ProtocolError);
}

void test_reject_wrong_schema_version() {
    const std::string bad = R"({
        "schema_version": 2,
        "repository_root": ".",
        "language": "cpp",
        "libraries": [],
        "solutions": []
    })";
    CE_EXPECT_THROWS(ce_cpp::parse_request(bad), ce_cpp::ProtocolError);
}

void test_reject_missing_required_key() {
    const std::string bad = R"({
        "schema_version": 1,
        "language": "cpp",
        "libraries": [],
        "solutions": []
    })";
    CE_EXPECT_THROWS(ce_cpp::parse_request(bad), ce_cpp::ProtocolError);
}

void test_reject_non_empty_libraries() {
    const std::string bad = R"({
        "schema_version": 1,
        "repository_root": ".",
        "language": "cpp",
        "libraries": [{"path": "x"}],
        "solutions": []
    })";
    CE_EXPECT_THROWS(ce_cpp::parse_request(bad), ce_cpp::ProtocolError);
}

void test_reject_unescaped_control_character() {
    // A literal newline (0x0A) inside a string is not permitted by JSON. The
    // strict parser must surface a ProtocolError instead of quietly accepting
    // the byte.
    std::string bad = "{\"schema_version\": 1, \"repository_root\": \"a\nb\", \"language\": \"cpp\", \"libraries\": [], \"solutions\": []}";
    CE_EXPECT_THROWS(ce_cpp::parse_request(bad), ce_cpp::ProtocolError);
}

void test_roundtrip_response() {
    ce_cpp::AnalysisResponse resp;
    resp.schema_version = ce_cpp::SCHEMA_VERSION;
    resp.adapter.name = "ce-cpp";
    resp.adapter.version = "0.1.0";
    ce_cpp::ToolchainIdentity clang;
    clang.name = "clang";
    clang.version = "22.1.0";
    clang.target = std::string("x86_64-unknown-linux-gnu");
    resp.adapter.toolchains.push_back(clang);

    const std::string json = ce_cpp::serialize_response(resp);
    const auto parsed = ce_cpp::parse_response(json);
    CE_EXPECT_EQ(parsed.schema_version, resp.schema_version);
    CE_EXPECT_EQ(parsed.adapter.name, resp.adapter.name);
    CE_EXPECT_EQ(parsed.adapter.version, resp.adapter.version);
    CE_EXPECT_EQ(parsed.adapter.toolchains.size(), size_t{1});
    CE_EXPECT_EQ(parsed.adapter.toolchains[0].name, std::string("clang"));
    CE_EXPECT_EQ(parsed.adapter.toolchains[0].version, std::string("22.1.0"));
    CE_EXPECT_TRUE(parsed.adapter.toolchains[0].target.has_value());
    CE_EXPECT_EQ(*parsed.adapter.toolchains[0].target,
                 std::string("x86_64-unknown-linux-gnu"));
}

}  // namespace

int main() {
    test_parse_empty_request_fixture();
    test_reject_unknown_key();
    test_reject_wrong_schema_version();
    test_reject_missing_required_key();
    test_reject_non_empty_libraries();
    test_reject_unescaped_control_character();
    test_roundtrip_response();
    if (failures > 0) {
        std::cerr << failures << " C++ handshake assertions failed" << std::endl;
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
