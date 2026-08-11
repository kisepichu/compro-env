// Adapter protocol v1 bindings for the ce-cpp analyzer (spec §§6.7, 6.9; plan
// 045 Task 2).
//
// The core owns the canonical protocol definitions in
// `crates/library-adapter-protocol`. These C++ bindings mirror only the
// fields that the empty handshake exercises: schema version, adapter
// identity, and the (currently empty) library/solution arrays. Dependency and
// symbol payloads land in later plans.
//
// The parser is strict: unknown top-level keys, wrong `schema_version`,
// non-array `libraries`/`solutions`, or non-empty payloads are all rejected.
// This matches the Rust adapter's `#[serde(deny_unknown_fields)]` posture and
// makes the handshake fail loudly if the core ever sends a v2 request to a
// v1 adapter by mistake.

#ifndef CE_CPP_PROTOCOL_HPP
#define CE_CPP_PROTOCOL_HPP

#include <cstdint>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>

namespace ce_cpp {

/// Adapter protocol version implemented by this executable.
constexpr uint32_t SCHEMA_VERSION = 1;
/// Adapter identity name reported at handshake.
constexpr std::string_view ADAPTER_NAME = "ce-cpp";
/// Adapter identity version reported at handshake.
constexpr std::string_view ADAPTER_VERSION = "0.1.0";

/// Observed identity of one toolchain used by this analyzer.
struct ToolchainIdentity {
    std::string name;
    std::string version;
    std::optional<std::string> target;
};

/// Analyzer self-identity plus observed toolchains.
struct AdapterIdentity {
    std::string name;
    std::string version;
    std::vector<ToolchainIdentity> toolchains;
};

/// Empty-handshake AnalysisRequest. `libraries` and `solutions` are deferred to
/// later plans; here we only assert they are empty arrays.
struct AnalysisRequest {
    uint32_t schema_version = 0;
    std::string repository_root;
    std::string language;
};

/// Empty-handshake AnalysisResponse. See the note on `AnalysisRequest` for the
/// deferred fields.
struct AnalysisResponse {
    uint32_t schema_version = 0;
    AdapterIdentity adapter;
};

/// Raised on any parser or serializer failure. The message is human-readable
/// but does not embed absolute paths per spec §6.3.
class ProtocolError : public std::runtime_error {
   public:
    using std::runtime_error::runtime_error;
};

/// Parse an `AnalysisRequest` JSON document. Throws `ProtocolError` if the
/// document violates the strict shape.
AnalysisRequest parse_request(std::string_view raw);

/// Serialize an `AnalysisResponse` to a single-line JSON document. The field
/// order matches the Rust adapter's output shape so byte-for-byte fixtures
/// stay stable across languages.
std::string serialize_response(const AnalysisResponse& response);

/// Parse an `AnalysisResponse` JSON document. Only used by tests; the runtime
/// never consumes its own output.
AnalysisResponse parse_response(std::string_view raw);

}  // namespace ce_cpp

#endif  // CE_CPP_PROTOCOL_HPP
