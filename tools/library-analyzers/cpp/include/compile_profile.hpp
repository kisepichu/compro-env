// Compile-profile parsing for the ce-cpp analyzer (spec §6.7; plan 046 Task 1).
//
// The checked-in `tools/library-analyzers/cpp/compile-profile.toml` file
// declares the C++ standard, preprocessor defines, and include roots that both
// the analyzer and `check_command` use. Parsing is strict: unknown or
// duplicated keys, missing required keys, non-repo-relative paths, and paths
// that escape the repository root all raise `CompileProfileError`.
//
// `buildClangArguments` produces the argv used to configure a Clang
// LibTooling invocation. The order is deterministic so fixture assertions can
// compare byte-for-byte.

#ifndef CE_CPP_COMPILE_PROFILE_HPP
#define CE_CPP_COMPILE_PROFILE_HPP

#include <filesystem>
#include <stdexcept>
#include <string>
#include <vector>

namespace ce_cpp {

/// Parsed representation of `compile-profile.toml`.
///
/// `include_roots` are stored as absolute, symlink-resolved paths under the
/// repository root, in the order they appeared in the TOML file. `defines` are
/// stored verbatim so that `NAME` and `NAME=VALUE` forms both pass through.
struct CompileProfile {
    std::string cxx_standard;
    std::vector<std::string> defines;
    std::vector<std::filesystem::path> include_roots;
    std::filesystem::path repository_root;
};

/// One analysis target passed to `buildClangArguments`.
struct Target {
    /// Absolute path to the translation-unit source file. Must live under
    /// `CompileProfile::repository_root`.
    std::filesystem::path source_file;
};

/// Raised on any compile-profile parsing or validation error.
class CompileProfileError : public std::runtime_error {
   public:
    using std::runtime_error::runtime_error;
};

/// Read `<repositoryRoot>/tools/library-analyzers/cpp/compile-profile.toml`
/// and return a validated `CompileProfile`. Throws `CompileProfileError` on
/// any parse or validation failure.
///
/// Environment variables such as `CXX`, `CXXFLAGS`, or `CPATH` are never
/// consulted — the profile is fully determined by the on-disk file plus the
/// repository root argument.
CompileProfile loadCompileProfile(const std::filesystem::path& repositoryRoot);

/// Build the deterministic argv used to configure a Clang LibTooling
/// invocation for one translation-unit target.
///
/// The order is: `-x c++`, `-std=<cxx_standard>`, `-D<define>` for each
/// declared define, `-I<absolute include root>` for each declared root, and
/// finally the absolute source file path. Callers that consume the argv as a
/// `char *const []` (LibTooling) do not need to add anything else.
std::vector<std::string> buildClangArguments(const CompileProfile& profile, const Target& target);

}  // namespace ce_cpp

#endif  // CE_CPP_COMPILE_PROFILE_HPP
