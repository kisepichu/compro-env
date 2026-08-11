// Direct-dependency analysis for the ce-cpp adapter (spec §6.7; plan 046
// Task 2).
//
// `analyze_target` runs Clang's preprocessor over one translation-unit
// target under the compile profile from plan 046 Task 1. It inspects the
// preprocessor's `InclusionDirective` callback, filters to includes issued
// directly from the target's main file (so nested/transitive includes do not
// become direct edges), and classifies each one:
//
//   * `internal`   — resolves to a repository-relative path present in the
//                    manifest set the caller passes in;
//   * `external`   — resolves outside the repository (system header) or
//                    inside the repository but never as a managed source
//                    (which would fall through to `unresolved`);
//   * `unresolved` — missing header, macro-expanded include name, or a
//                    resolved path inside the repository that is not part
//                    of the manifest set. `state = partial` fires whenever
//                    at least one unresolved edge exists.
//
// The public interface only exposes a `TargetOutcome`; the caller (main
// binary or test) then folds that into a `LibraryAnalysis` or
// `SolutionAnalysis`.

#ifndef CE_CPP_DEPENDENCIES_HPP
#define CE_CPP_DEPENDENCIES_HPP

#include <filesystem>
#include <string>
#include <vector>

#include "compile_profile.hpp"
#include "protocol.hpp"

namespace ce_cpp {

/// Set of managed library paths, repo-relative, forward-slash separated,
/// as they appear on the wire in `AnalysisRequest.libraries[].path`.
using ManifestSet = std::vector<std::string>;

/// Result of analyzing one translation-unit target. Ordering of
/// `dependencies` follows source order (line, then column) since the
/// preprocessor lexes the main file top-to-bottom.
struct TargetOutcome {
    std::vector<Dependency> dependencies;
    AnalysisState state = AnalysisState::Complete;
    std::vector<Diagnostic> diagnostics;
};

/// Analyze one target file. `source_file` must be absolute and live under
/// `profile.repository_root`. `target_path` is the repo-relative path
/// filled into each edge's `Location.path` (matches the LibraryTarget path
/// or SolutionTarget entry the request supplied).
TargetOutcome analyze_target(const CompileProfile& profile,
                             const std::filesystem::path& source_file,
                             const std::string& target_path,
                             const ManifestSet& manifest);

}  // namespace ce_cpp

#endif  // CE_CPP_DEPENDENCIES_HPP
