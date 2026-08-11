// Adapter protocol v1 bindings for the ce-cpp analyzer (spec §§6.7, 6.9;
// plans 045 & 046).
//
// The core owns the canonical protocol definitions in
// `crates/library-adapter-protocol`. These C++ bindings mirror the fields
// needed by the empty handshake (plan 045) and the direct-dependency response
// (plan 046). Symbol payloads land in plan 047.
//
// The parser is strict: unknown top-level keys, wrong `schema_version`, or
// unrecognized enum tokens are all rejected. That matches the Rust adapter's
// `#[serde(deny_unknown_fields)]` posture and makes the handshake fail loudly
// if the core ever sends a v2 request to a v1 adapter by mistake.

#ifndef CE_CPP_PROTOCOL_HPP
#define CE_CPP_PROTOCOL_HPP

#include <cstdint>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace ce_cpp {

/// Adapter protocol version implemented by this executable.
constexpr uint32_t SCHEMA_VERSION = 1;
/// Adapter identity name reported at handshake.
constexpr std::string_view ADAPTER_NAME = "ce-cpp";
/// Adapter identity version reported at handshake.
constexpr std::string_view ADAPTER_VERSION = "0.1.0";

// ─── Toolchain / adapter identity ───────────────────────────────────────────

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

// ─── Request payload ────────────────────────────────────────────────────────

/// One managed library file the core asked us to analyze.
struct LibraryTarget {
    std::string path;
};

/// One solution target the core asked us to analyze.
struct SolutionTarget {
    std::string id;
    std::string root;
    std::string entry;
};

/// Deserialized `AnalysisRequest` document.
struct AnalysisRequest {
    uint32_t schema_version = 0;
    std::string repository_root;
    std::string language;
    std::vector<LibraryTarget> libraries;
    std::vector<SolutionTarget> solutions;
};

// ─── Response payload ───────────────────────────────────────────────────────

/// Byte-oriented one-based source position. Both fields are 1-indexed per the
/// shared protocol schema; the `column` field is optional to accommodate
/// callers that only know a line.
struct Position {
    uint32_t line = 0;
    std::optional<uint32_t> column;
};

/// Source location covering a directive or a symbol. `end` is optional so
/// callers can report just a start position.
struct Location {
    std::string path;
    Position start;
    std::optional<Position> end;
};

enum class DependencyKind {
    Internal,
    External,
    Unresolved,
};

/// One direct-dependency edge. Fields are populated based on `kind`:
///
///   * `Internal`  → `path` (repo-relative target file).
///   * `External`  → `name` (external package or system header identifier).
///   * `Unresolved` → `key` + `display` (stable diagnostic pair).
struct Dependency {
    DependencyKind kind = DependencyKind::Internal;
    std::optional<Location> location;
    std::string path;
    std::string name;
    std::string key;
    std::string display;
};

enum class AnalysisState {
    Complete,
    Partial,
    Failed,
};

struct DependencyAnalysis {
    AnalysisState state = AnalysisState::Complete;
    std::vector<Dependency> dependencies;
};

struct SymbolAnalysis {
    AnalysisState state = AnalysisState::Partial;
    // Symbols land in plan 047; the C++ adapter always emits an empty list
    // for now with a `partial` state.
};

enum class Severity {
    Info,
    Warning,
    Error,
};

struct Diagnostic {
    Severity severity = Severity::Error;
    std::string code;
    std::string message;
    std::optional<Location> location;
};

struct LibraryAnalysis {
    std::string path;
    DependencyAnalysis dependency_analysis;
    SymbolAnalysis symbol_analysis;
    std::vector<Diagnostic> diagnostics;
};

struct SolutionAnalysis {
    std::string id;
    DependencyAnalysis dependency_analysis;
    std::vector<Diagnostic> diagnostics;
};

struct AnalysisResponse {
    uint32_t schema_version = 0;
    AdapterIdentity adapter;
    std::vector<LibraryAnalysis> libraries;
    std::vector<SolutionAnalysis> solutions;
};

// ─── Error type ─────────────────────────────────────────────────────────────

/// Raised on any parser or serializer failure. The message is human-readable
/// but does not embed absolute paths per spec §6.3.
class ProtocolError : public std::runtime_error {
   public:
    using std::runtime_error::runtime_error;
};

// ─── Parser / serializer entry points ───────────────────────────────────────

/// Parse an `AnalysisRequest` JSON document. Throws `ProtocolError` if the
/// document violates the strict shape.
AnalysisRequest parse_request(std::string_view raw);

/// Serialize an `AnalysisResponse` to a pretty-printed JSON document with
/// alphabetical keys and 2-space indent — matching the Rust adapter's
/// `serde_json::to_string_pretty` output so shared fixtures stay
/// byte-for-byte comparable.
std::string serialize_response(const AnalysisResponse& response);

/// Parse an `AnalysisResponse` JSON document. Used by tests and by other
/// tools that consume adapter output.
AnalysisResponse parse_response(std::string_view raw);

}  // namespace ce_cpp

#endif  // CE_CPP_PROTOCOL_HPP
