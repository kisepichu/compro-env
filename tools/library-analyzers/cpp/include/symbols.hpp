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
// `AnalysisState` to `Partial`. A hard AST failure (impossible under the
// caller's driver, which is a `SyntaxOnlyAction` we control) yields `Failed`
// with an empty symbol list — callers can distinguish that from partial via
// the state.

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
/// Clang's syntax-only action and run `analyzeSymbols` on the resulting AST.
/// Returns `Failed` with an empty list if the frontend refused to build an
/// AST for the file at all (missing file, cataclysmic driver error). Recovery
/// from partial parses stays at `Complete` for the symbols we could still
/// pin, and the caller degrades the state elsewhere if needed.
SymbolOutcome analyzeSymbolsForTarget(const CompileProfile& profile,
                                      const std::filesystem::path& source_file,
                                      const std::string& target_path);

}  // namespace ce_cpp

#endif  // CE_CPP_SYMBOLS_HPP
