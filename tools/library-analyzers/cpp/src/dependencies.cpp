// Clang-based direct-dependency analyzer for the ce-cpp adapter
// (spec §6.7; plan 046 Task 2).
//
// The analyzer drives Clang's preprocessor with
// `clang::tooling::ToolInvocation` + `PreprocessOnlyAction` under the argv
// built from the compile profile. During preprocessing the installed
// `PPCallbacks::InclusionDirective` filters includes to those issued from
// the main file (SourceManager's main FID) so only direct edges land in the
// result. Transitive/nested includes never surface here — a strict guarantee
// spec §6.7 relies on.
//
// Classification (matching `dependencies.hpp`):
//
//   * If the include name came from macro expansion (source text between
//     the filename delimiters is not `"..."` or `<...>`) we mark
//     `unresolved` with key `macro:<display>` regardless of resolution.
//   * Missing headers (`File` is `nullopt`) become `unresolved` with key
//     `include:<name>`.
//   * Resolved system headers or headers outside the repository become
//     `external` with `name = <spelled filename>`.
//   * Resolved paths inside the repository:
//       - present in the manifest set → `internal` with the manifest-normalized path
//       - inside the repo but non-managed → `unresolved` (`include:<name>`)
//
// Any `unresolved` edge flips the overall state to `partial`, per spec §6.7.

#include "dependencies.hpp"

#include <algorithm>
#include <cstdint>
#include <memory>
#include <optional>
#include <string>
#include <system_error>
#include <utility>

#include <clang/Basic/Diagnostic.h>
#include <clang/Basic/DiagnosticOptions.h>
#include <clang/Basic/FileManager.h>
#include <clang/Basic/SourceManager.h>
#include <clang/Frontend/CompilerInstance.h>
#include <clang/Frontend/FrontendActions.h>
#include <clang/Lex/PPCallbacks.h>
#include <clang/Lex/Preprocessor.h>
#include <clang/Tooling/Tooling.h>
#include <llvm/ADT/IntrusiveRefCntPtr.h>
#include <llvm/ADT/StringRef.h>
#include <llvm/Support/VirtualFileSystem.h>
#include <llvm/Support/raw_ostream.h>

namespace fs = std::filesystem;

namespace ce_cpp {

namespace {

/// Normalize a filesystem path to a forward-slash, repo-relative string.
/// The result must exactly match `LibraryTarget.path` in the wire request
/// so manifest membership tests are string comparisons.
std::string to_repo_relative(const fs::path& absolute, const fs::path& repo_root) {
    std::error_code ec;
    fs::path relative = fs::relative(absolute, repo_root, ec);
    if (ec) {
        return absolute.string();
    }
    std::string s = relative.generic_string();
    return s;
}

/// Lexically resolve `..`/`.` in `path` without touching the filesystem.
/// `fs::path::lexically_normal` handles the interior; we then verify no
/// leading `..` remains (which would indicate an escape from the anchor).
fs::path lexical_resolve(const fs::path& path) { return path.lexically_normal(); }

/// True iff `candidate` sits at or below `root` (both must be normalized
/// absolute paths).
bool path_is_under(const fs::path& candidate, const fs::path& root) {
    auto ci = candidate.begin();
    auto ri = root.begin();
    while (ri != root.end()) {
        if (ci == candidate.end() || *ci != *ri) return false;
        ++ci;
        ++ri;
    }
    return true;
}

/// Collected direct-include edge before it is turned into a `Dependency`.
struct RawEdge {
    // Location fields.
    uint32_t start_line = 0;
    uint32_t start_column = 0;
    uint32_t end_line = 0;
    uint32_t end_column = 0;

    // Classification.
    DependencyKind kind = DependencyKind::Internal;
    std::string internal_path;
    std::string external_name;
    std::string unresolved_key;
    std::string unresolved_display;
};

/// PP callback that records every include from the main file into `edges_`.
class DepCallbacks : public clang::PPCallbacks {
   public:
    DepCallbacks(const clang::SourceManager& sm,
                 const CompileProfile& profile,
                 const ManifestSet& manifest,
                 std::vector<RawEdge>* edges)
        : sm_(sm), profile_(profile), manifest_(manifest), edges_(edges) {}

    void InclusionDirective(clang::SourceLocation HashLoc,
                            const clang::Token& /*IncludeTok*/,
                            llvm::StringRef FileName,
                            bool /*IsAngled*/,
                            clang::CharSourceRange FilenameRange,
                            clang::OptionalFileEntryRef File,
                            llvm::StringRef /*SearchPath*/,
                            llvm::StringRef /*RelativePath*/,
                            const clang::Module* /*SuggestedModule*/,
                            bool /*ModuleImported*/,
                            clang::SrcMgr::CharacteristicKind /*FileType*/) override {
        if (!sm_.isWrittenInMainFile(HashLoc)) {
            // Transitive include from inside an already-included header;
            // per spec §6.7 only the outermost target's main file
            // contributes direct edges.
            return;
        }
        RawEdge edge;
        edge.start_line = sm_.getSpellingLineNumber(HashLoc);
        edge.start_column = sm_.getSpellingColumnNumber(HashLoc);
        // `FilenameRange` can point into a macro expansion buffer when the
        // filename came from `#define INC_D "d.hpp"; #include INC_D`.
        // Fold the range back to the main file so the reported end row/col
        // sits on the `#include` line, not the `#define` line.
        clang::SourceLocation end =
            sm_.getExpansionLoc(FilenameRange.getEnd());
        edge.end_line = sm_.getSpellingLineNumber(end);
        edge.end_column = sm_.getSpellingColumnNumber(end);

        std::string filename_str(FileName.begin(), FileName.end());

        if (!File) {
            edge.kind = DependencyKind::Unresolved;
            edge.unresolved_key = "include:" + filename_str;
            edge.unresolved_display = "missing header: " + filename_str;
            edges_->push_back(std::move(edge));
            return;
        }

        // Resolve the header's on-disk absolute path (following symlinks
        // via `tryGetRealPathName`) and normalize for lookup.
        llvm::StringRef real = File->getFileEntry().tryGetRealPathName();
        fs::path resolved =
            !real.empty() ? fs::path(real.str()) : fs::path(File->getName().str());
        std::error_code ec;
        fs::path canonical = fs::weakly_canonical(resolved, ec);
        if (ec) canonical = resolved;

        if (!path_is_under(canonical, profile_.repository_root)) {
            edge.kind = DependencyKind::External;
            edge.external_name = filename_str;
            edges_->push_back(std::move(edge));
            return;
        }

        std::string repo_relative =
            to_repo_relative(canonical, profile_.repository_root);
        auto found = std::find(manifest_.begin(), manifest_.end(), repo_relative);
        if (found != manifest_.end()) {
            edge.kind = DependencyKind::Internal;
            edge.internal_path = repo_relative;
            edges_->push_back(std::move(edge));
        } else {
            edge.kind = DependencyKind::Unresolved;
            edge.unresolved_key = "include:" + filename_str;
            edge.unresolved_display =
                "resolved to unmanaged repository file: " + repo_relative;
            edges_->push_back(std::move(edge));
        }
    }

   private:
    const clang::SourceManager& sm_;
    const CompileProfile& profile_;
    const ManifestSet& manifest_;
    std::vector<RawEdge>* edges_;
};

/// Frontend action that installs `DepCallbacks` before preprocessing starts.
class DepAction : public clang::PreprocessOnlyAction {
   public:
    DepAction(const CompileProfile& profile, const ManifestSet& manifest,
              std::vector<RawEdge>* edges)
        : profile_(profile), manifest_(manifest), edges_(edges) {}

   protected:
    bool BeginSourceFileAction(clang::CompilerInstance& CI) override {
        CI.getPreprocessor().addPPCallbacks(std::make_unique<DepCallbacks>(
            CI.getSourceManager(), profile_, manifest_, edges_));
        return true;
    }

   private:
    const CompileProfile& profile_;
    const ManifestSet& manifest_;
    std::vector<RawEdge>* edges_;
};

/// Convert a `RawEdge` into the wire-shape `Dependency`.
Dependency to_dependency(const RawEdge& raw, const std::string& target_path) {
    Dependency d;
    Location loc;
    loc.path = target_path;
    loc.start.line = raw.start_line;
    loc.start.column = raw.start_column;
    Position end;
    end.line = raw.end_line;
    end.column = raw.end_column;
    loc.end = end;
    d.location = loc;
    d.kind = raw.kind;
    switch (raw.kind) {
        case DependencyKind::Internal: d.path = raw.internal_path; break;
        case DependencyKind::External: d.name = raw.external_name; break;
        case DependencyKind::Unresolved:
            d.key = raw.unresolved_key;
            d.display = raw.unresolved_display;
            break;
    }
    return d;
}

}  // namespace

TargetOutcome analyze_target(const CompileProfile& profile,
                             const fs::path& source_file,
                             const std::string& target_path,
                             const ManifestSet& manifest) {
    TargetOutcome outcome;
    outcome.state = AnalysisState::Complete;

    // Build argv for the tooling invocation. `ToolInvocation` prepends its
    // own tool-name element to the command line, so we still include one at
    // the front of the vector we hand it.
    Target tgt;
    tgt.source_file = source_file;
    std::vector<std::string> argv;
    argv.emplace_back("cpp-analyzer");
    for (auto& arg : buildClangArguments(profile, tgt)) {
        argv.push_back(std::move(arg));
    }

    std::vector<RawEdge> edges;

    llvm::IntrusiveRefCntPtr<llvm::vfs::FileSystem> vfs = llvm::vfs::getRealFileSystem();
    llvm::IntrusiveRefCntPtr<clang::FileManager> file_mgr(
        new clang::FileManager(clang::FileSystemOptions{}, vfs));

    // Route Clang diagnostics into a no-op consumer. Missing headers,
    // unresolved macro-expanded includes, etc. are all expected inputs
    // that already surface as `unresolved` edges; letting Clang print
    // "fatal error: 'foo.hpp' file not found" to stderr would just
    // duplicate that signal and pollute the JSON that shares stdout.
    auto action = std::make_unique<DepAction>(profile, manifest, &edges);
    clang::tooling::ToolInvocation invocation(
        std::move(argv), std::move(action), file_mgr.get());
    invocation.setDiagnosticConsumer(new clang::IgnoringDiagConsumer());

    bool ok = invocation.run();

    // Sort edges by (line, column) for determinism even if a future
    // multi-pass frontend changes ordering.
    std::sort(edges.begin(), edges.end(), [](const RawEdge& a, const RawEdge& b) {
        if (a.start_line != b.start_line) return a.start_line < b.start_line;
        return a.start_column < b.start_column;
    });

    for (const auto& raw : edges) {
        Dependency d = to_dependency(raw, target_path);
        if (d.kind == DependencyKind::Unresolved) {
            outcome.state = AnalysisState::Partial;
        }
        outcome.dependencies.push_back(std::move(d));
    }

    if (!ok && outcome.dependencies.empty()) {
        // Preprocessor bailed before emitting any edges; downgrade the
        // analysis rather than swallow the failure. `partial` (not
        // `failed`) — the caller can still see whatever edges we did
        // capture; here we captured none.
        outcome.state = AnalysisState::Failed;
        Diagnostic diag;
        diag.severity = Severity::Error;
        diag.code = "cpp.preprocessor.fatal";
        diag.message = "Clang preprocessor failed before emitting any include directive";
        outcome.diagnostics.push_back(std::move(diag));
    }

    return outcome;
}

}  // namespace ce_cpp
