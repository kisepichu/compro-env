// AST-driven symbol projection for the ce-cpp adapter (spec §§6.7, 12.5,
// 13.1; plan 047).
//
// `analyzeSymbols` walks a fully-parsed `ASTContext` and returns every
// declaration whose spelling location lives inside the requested managed
// source file. C++ syntax stays adapter-private: the returned `Symbol`
// values only carry stable adapter tokens (`class`, `function`, `method`,
// `concept`, `type`, `value`) plus qualified/search names and 1-based
// spelling-location ranges. The core layer never interprets these tokens.
//
// The visitor filters out declarations that:
//
//   * originate outside the target file (system / other-repo headers);
//   * are implicit compiler-injected declarations;
//   * have unusable source ranges (invalid or reversed).
//
// Filtered-but-still-parsed declarations degrade the returned
// `AnalysisState` to `Partial`. A hard driver failure — the frontend refused
// to build an AST at all — yields `Failed` with an empty symbol list;
// callers distinguish that from partial via the state. Driver-reported
// errors that still left some declarations parsed also degrade to `Partial`
// so recovery output stays visible.

#ifndef CE_CPP_SYMBOLS_HPP
#define CE_CPP_SYMBOLS_HPP

#include <filesystem>

#include "compile_profile.hpp"
#include "protocol.hpp"

namespace clang {
class ASTContext;
}  // namespace clang

namespace ce_cpp {

/// Result of analyzing symbols in one target. Diagnostics stay empty at this
/// layer; the caller composes them alongside dependency diagnostics.
struct SymbolOutcome {
    SymbolAnalysis analysis;
};

/// Analyze the declarations belonging to `managedSource` in `context`.
///
/// * `context` — a fully-parsed AST context whose main-file source manager
///   already points at `managedSource`.
/// * `managedSource` — absolute path to the file whose declarations we
///   want to project. Only declarations whose spelling location resolves
///   into this file are emitted.
/// * `target_path` — repo-relative POSIX path stamped into every emitted
///   `Location.path`.
SymbolOutcome analyzeSymbols(clang::ASTContext& context,
                             const std::filesystem::path& managedSource,
                             const std::string& target_path);

/// Convenience end-to-end driver: parse `source_file` under `profile` with
/// an `ASTFrontendAction` and run the collector on the resulting AST.
/// Returns `Failed` with an empty list if the frontend refused to build an
/// AST for the file at all (missing file, cataclysmic driver error).
/// Driver errors that still yielded some declarations downgrade to
/// `Partial` so partial-catalog output stays visible.
SymbolOutcome analyzeSymbolsForTarget(const CompileProfile& profile,
                                      const std::filesystem::path& source_file,
                                      const std::string& target_path);

}  // namespace ce_cpp

#endif  // CE_CPP_SYMBOLS_HPP
