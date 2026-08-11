// AST-driven symbol projection for the ce-cpp adapter (spec §§6.7, 12.5,
// 13.1; plan 047).
//
// The visitor walks the AST built by Clang's `SyntaxOnlyAction` under the
// caller's compile profile and emits one `Symbol` per declaration whose
// spelling location resolves into the requested managed source file.
//
// Design notes
// ------------
//
// * We walk the declaration tree manually (via `TranslationUnitDecl::decls`)
//   rather than lean on `RecursiveASTVisitor`. Manual walking keeps ordering
//   deterministic and lets us skip declarations that are compiler-injected
//   (`Decl::isImplicit`), that live in `LinkageSpecDecl` bodies we do not
//   emit for, or that are the "not first" declaration of a redeclaration
//   chain (forward decls of the same entity).
//
// * `SourceManager::isWrittenInMainFile` on the spelling location is the
//   filter that guarantees only declarations physically typed inside
//   `managedSource` are emitted. Macro-expanded declarations whose macro
//   *definition* lives in an included header have their spelling location
//   *inside that header*, and are correctly filtered out.
//
// * Redeclaration handling: we key on `Decl::getCanonicalDecl()` and emit
//   at most once per canonical declaration. Preference order for the emit
//   location:
//     1. the first declaration in the main file that `isThisDeclarationADefinition()`;
//     2. otherwise the first declaration in the main file.
//   This gives forward declarations a stable single symbol without dropping
//   any that only ever forward-declare.
//
// * Overloads are distinct canonical decls, so both are emitted with the
//   same qualified name but different locations. `signature` carries the
//   pretty-printed parameter list so search results can differentiate.
//
// * Kind vocabulary (plan 047 constraint): `class`, `function`, `method`,
//   `concept`, `type`, `value`. Namespaces, typedefs, aliases, and enums map
//   to `type`; ctors/dtors/operators/methods to `method`; enumerators and
//   variables to `value`.

#include "symbols.hpp"

#include <algorithm>
#include <cstdint>
#include <memory>
#include <string>
#include <system_error>
#include <unordered_set>
#include <utility>

#include <clang/AST/ASTContext.h>
#include <clang/AST/Decl.h>
#include <clang/AST/DeclBase.h>
#include <clang/AST/DeclCXX.h>
#include <clang/AST/DeclTemplate.h>
#include <clang/AST/PrettyPrinter.h>
#include <clang/Basic/Diagnostic.h>
#include <clang/Basic/FileManager.h>
#include <clang/Basic/SourceLocation.h>
#include <clang/Basic/SourceManager.h>
#include <clang/Frontend/CompilerInstance.h>
#include <clang/Frontend/FrontendActions.h>
#include <clang/Lex/Lexer.h>
#include <clang/Tooling/Tooling.h>
#include <llvm/ADT/IntrusiveRefCntPtr.h>
#include <llvm/ADT/SmallString.h>
#include <llvm/ADT/StringRef.h>
#include <llvm/Support/VirtualFileSystem.h>
#include <llvm/Support/raw_ostream.h>

namespace fs = std::filesystem;

namespace ce_cpp {

namespace {

// ─── Kind tags (adapter-private tokens per spec §6.7 / plan 047) ────────────

constexpr const char* KIND_CLASS = "class";
constexpr const char* KIND_FUNCTION = "function";
constexpr const char* KIND_METHOD = "method";
constexpr const char* KIND_CONCEPT = "concept";
constexpr const char* KIND_TYPE = "type";
constexpr const char* KIND_VALUE = "value";

// ─── Raw symbol staging record ──────────────────────────────────────────────

/// Symbol staged before serialization. Stores raw ints so we can sort by
/// (line, column) deterministically without dragging `SourceLocation` through
/// downstream code.
struct RawSymbol {
    std::string name;
    std::string kind;
    std::string qualified_name;
    std::string signature;
    uint32_t start_line = 0;
    uint32_t start_column = 0;
    uint32_t end_line = 0;
    uint32_t end_column = 0;
    bool has_end = false;
};

// ─── AST location utilities ─────────────────────────────────────────────────

/// True iff the spelling of `loc` sits inside `SM`'s main file. Guards every
/// per-decl emit so system headers, other included managed files, and
/// macro definitions from other translation units all fall out.
bool spelling_in_main_file(const clang::SourceManager& SM, clang::SourceLocation loc) {
    if (loc.isInvalid()) return false;
    clang::SourceLocation spelling = SM.getSpellingLoc(loc);
    if (spelling.isInvalid()) return false;
    return SM.isWrittenInMainFile(spelling);
}

/// Extract 1-based line/column at the spelling location of `loc`. Returns
/// (0, 0) when the location has no usable coordinates — the caller uses
/// (line == 0) as the "unusable" sentinel.
std::pair<uint32_t, uint32_t> spelling_line_col(const clang::SourceManager& SM,
                                                clang::SourceLocation loc) {
    if (loc.isInvalid()) return {0, 0};
    clang::SourceLocation spelling = SM.getSpellingLoc(loc);
    unsigned line = SM.getSpellingLineNumber(spelling);
    unsigned col = SM.getSpellingColumnNumber(spelling);
    return {static_cast<uint32_t>(line), static_cast<uint32_t>(col)};
}

/// End-of-token span for `end`. Clang's `getEndLoc()` points at the start of
/// the last token; `Lexer::getLocForEndOfToken` advances past it so callers
/// get an exclusive end column.
clang::SourceLocation end_of_token(const clang::SourceManager& SM,
                                   const clang::LangOptions& LO,
                                   clang::SourceLocation end) {
    if (end.isInvalid()) return end;
    return clang::Lexer::getLocForEndOfToken(end, /*Offset=*/0, SM, LO);
}

// ─── Kind classification ────────────────────────────────────────────────────

const char* function_kind(const clang::FunctionDecl* FD) {
    return llvm::isa<clang::CXXMethodDecl>(FD) ? KIND_METHOD : KIND_FUNCTION;
}

/// Determine the adapter kind token for a given decl node. Returns nullptr
/// when the decl is not a category we emit (e.g. `FieldDecl`, `ParmVarDecl`,
/// or a `UsingDirectiveDecl`).
const char* classify(const clang::Decl* D) {
    if (llvm::isa<clang::NamespaceDecl>(D)) return KIND_TYPE;
    if (llvm::isa<clang::EnumDecl>(D)) return KIND_TYPE;
    if (llvm::isa<clang::EnumConstantDecl>(D)) return KIND_VALUE;
    if (llvm::isa<clang::TypedefDecl>(D)) return KIND_TYPE;
    if (llvm::isa<clang::TypeAliasDecl>(D)) return KIND_TYPE;
    if (llvm::isa<clang::TypeAliasTemplateDecl>(D)) return KIND_TYPE;
    if (llvm::isa<clang::ConceptDecl>(D)) return KIND_CONCEPT;
    if (const auto* R = llvm::dyn_cast<clang::CXXRecordDecl>(D)) {
        // Anonymous struct/union declarations (`union { int a; };`) have no
        // usable name and mostly serve to introduce fields. Drop them.
        if (R->getIdentifier() == nullptr && !R->isLambda() && !R->isAnonymousStructOrUnion()) {
            // Still emit if it has a typedef name (rare) — handled by name step.
        }
        return KIND_CLASS;
    }
    if (llvm::isa<clang::ClassTemplateDecl>(D)) return KIND_CLASS;
    if (llvm::isa<clang::CXXMethodDecl>(D)) return KIND_METHOD;
    if (const auto* FT = llvm::dyn_cast<clang::FunctionTemplateDecl>(D)) {
        return function_kind(FT->getTemplatedDecl());
    }
    if (llvm::isa<clang::FunctionDecl>(D)) {
        return function_kind(llvm::cast<clang::FunctionDecl>(D));
    }
    if (llvm::isa<clang::VarTemplateDecl>(D)) return KIND_VALUE;
    if (llvm::isa<clang::VarDecl>(D)) return KIND_VALUE;
    return nullptr;
}

// ─── Name helpers ───────────────────────────────────────────────────────────

/// Pretty-print a plain identifier, `operator+`, ctor/dtor, or conversion
/// name. `NamedDecl::getNameAsString` handles all of these — including
/// `~Type` for destructors and `operator T` for conversion functions — so
/// we forward the result verbatim.
std::string simple_name(const clang::NamedDecl* ND) {
    return ND->getNameAsString();
}

/// Return a stable qualified name. Clang formats anonymous namespaces as
/// `(anonymous namespace)::Item`; anonymous classes as `(anonymous class)::…`.
/// We also insert a stable location tag for a truly anonymous top-level
/// namespace so two files' `namespace {}` blocks do not collide in the
/// search index. That rule stays inside this file — the core never sees
/// C++-specific tokens.
std::string qualified_name(const clang::NamedDecl* ND) {
    std::string q;
    llvm::raw_string_ostream os(q);
    clang::PrintingPolicy pp(ND->getASTContext().getLangOpts());
    pp.SuppressUnwrittenScope = true;
    pp.SuppressScope = false;
    ND->printQualifiedName(os, pp);
    return os.str();
}

/// Pretty-print a function's parameter list for the `signature` field.
/// Return types are omitted so overload search still matches the intuitive
/// call syntax.
std::string function_signature(const clang::FunctionDecl* FD) {
    std::string sig;
    llvm::raw_string_ostream os(sig);
    clang::PrintingPolicy pp(FD->getASTContext().getLangOpts());
    pp.SuppressScope = true;
    pp.SuppressTagKeyword = true;
    os << "(";
    bool first = true;
    for (const auto* P : FD->parameters()) {
        if (!first) os << ", ";
        first = false;
        P->getType().print(os, pp);
    }
    os << ")";
    return os.str();
}

// ─── Redeclaration selection ────────────────────────────────────────────────

/// Pick the "best" declaration of a redeclaration chain to emit, restricted
/// to declarations in the main file. Preference: the first definition;
/// otherwise the first main-file declaration. Returns nullptr when none of
/// the chain sits in the main file.
const clang::Decl* pick_main_file_decl(const clang::Decl* D, const clang::SourceManager& SM) {
    const clang::Decl* first_main = nullptr;
    for (auto it = D->redecls_begin(); it != D->redecls_end(); ++it) {
        const clang::Decl* R = *it;
        if (!spelling_in_main_file(SM, R->getLocation())) continue;
        if (const auto* T = llvm::dyn_cast<clang::TagDecl>(R)) {
            if (T->isThisDeclarationADefinition()) return R;
        }
        if (const auto* F = llvm::dyn_cast<clang::FunctionDecl>(R)) {
            if (F->isThisDeclarationADefinition()) return R;
        }
        if (const auto* V = llvm::dyn_cast<clang::VarDecl>(R)) {
            if (V->isThisDeclarationADefinition()) return R;
        }
        if (first_main == nullptr) first_main = R;
    }
    return first_main;
}

// ─── Visitor ────────────────────────────────────────────────────────────────

/// Symbol collector. Walks decls in declared source order (that's the order
/// `DeclContext::decls` returns them in) and stages each accepted entity.
class SymbolCollector {
   public:
    SymbolCollector(clang::ASTContext& ctx, std::string target_path)
        : ctx_(ctx),
          sm_(ctx.getSourceManager()),
          langopts_(ctx.getLangOpts()),
          target_path_(std::move(target_path)) {}

    void run() {
        walk_decl_context(ctx_.getTranslationUnitDecl());
    }

    std::vector<RawSymbol>& symbols() { return symbols_; }
    bool has_dropped_location() const { return dropped_location_; }

   private:
    void walk_decl_context(const clang::DeclContext* DC) {
        for (const clang::Decl* D : DC->decls()) {
            visit(D);
        }
    }

    void visit(const clang::Decl* D) {
        if (D->isImplicit()) return;

        // `LinkageSpecDecl` (`extern "C" { ... }`) is transparent — descend
        // into its body but do not emit for the wrapper itself.
        if (const auto* L = llvm::dyn_cast<clang::LinkageSpecDecl>(D)) {
            walk_decl_context(L);
            return;
        }

        // Namespaces are both containers and named entities. Emit the
        // namespace symbol at its own location and recurse into its body.
        if (const auto* NS = llvm::dyn_cast<clang::NamespaceDecl>(D)) {
            handle_namespace(NS);
            return;
        }

        // Enums emit the enum type and every enumerator inside it. Recursion
        // through `walk_decl_context` would double-emit because the child
        // walker also visits enum-constant decls, so we intercept here.
        if (const auto* E = llvm::dyn_cast<clang::EnumDecl>(D)) {
            handle_enum(E);
            return;
        }

        // Templates: use the templated inner decl for location + name, but
        // classify by the wrapper. The wrapper carries the template
        // parameter clauses in its source range.
        if (const auto* CT = llvm::dyn_cast<clang::ClassTemplateDecl>(D)) {
            emit_named(CT, CT->getTemplatedDecl(), KIND_CLASS);
            // Recurse into the templated record body for nested members.
            if (const clang::CXXRecordDecl* R = CT->getTemplatedDecl();
                R != nullptr && R->isThisDeclarationADefinition()) {
                walk_record_body(R);
            }
            return;
        }
        if (const auto* FT = llvm::dyn_cast<clang::FunctionTemplateDecl>(D)) {
            emit_named(FT, FT->getTemplatedDecl(), function_kind(FT->getTemplatedDecl()),
                       function_signature(FT->getTemplatedDecl()));
            return;
        }
        if (const auto* AT = llvm::dyn_cast<clang::TypeAliasTemplateDecl>(D)) {
            emit_named(AT, AT->getTemplatedDecl(), KIND_TYPE);
            return;
        }
        if (const auto* VT = llvm::dyn_cast<clang::VarTemplateDecl>(D)) {
            emit_named(VT, VT->getTemplatedDecl(), KIND_VALUE);
            return;
        }

        // Class/struct/union declarations. Only emit for the "chosen" main-
        // file decl in the redecl chain; walking the body still happens once
        // per definition.
        if (const auto* R = llvm::dyn_cast<clang::CXXRecordDecl>(D)) {
            handle_record(R);
            return;
        }

        // Plain typedef / alias.
        if (llvm::isa<clang::TypedefDecl>(D) || llvm::isa<clang::TypeAliasDecl>(D)) {
            emit_named(D, llvm::cast<clang::NamedDecl>(D), KIND_TYPE);
            return;
        }

        // Concept.
        if (const auto* C = llvm::dyn_cast<clang::ConceptDecl>(D)) {
            emit_named(C, C, KIND_CONCEPT);
            return;
        }

        // Functions (free functions; methods are handled inside record body).
        if (const auto* FD = llvm::dyn_cast<clang::FunctionDecl>(D)) {
            emit_named(FD, FD, function_kind(FD), function_signature(FD));
            return;
        }

        // Variables (namespace-scope + top-level).
        if (const auto* V = llvm::dyn_cast<clang::VarDecl>(D)) {
            // Skip parameter variables — they arrive here only when the
            // caller feeds us a lambda body or nested block, but guard
            // anyway.
            if (llvm::isa<clang::ParmVarDecl>(V)) return;
            emit_named(V, V, KIND_VALUE);
            return;
        }
    }

    void handle_namespace(const clang::NamespaceDecl* NS) {
        // Only emit for the first main-file decl of the namespace so `namespace foo { ... }`
        // reopened in the same file yields one symbol.
        const clang::Decl* pick = pick_main_file_decl(NS, sm_);
        if (pick == NS) {
            emit_named(NS, NS, KIND_TYPE);
        }
        // Traverse this specific declaration's body — reopenings each carry
        // their own decls and would be lost if we always walked the "picked"
        // decl only.
        walk_decl_context(NS);
    }

    void handle_enum(const clang::EnumDecl* E) {
        const clang::Decl* pick = pick_main_file_decl(E, sm_);
        if (pick == E) {
            emit_named(E, E, KIND_TYPE);
        }
        // Enumerators live inside the definition — walk them only for the
        // definition to avoid re-emitting for forward `enum class X : int;`
        // declarations that carry no enumerators anyway.
        if (E->isThisDeclarationADefinition()) {
            for (const clang::EnumConstantDecl* EC : E->enumerators()) {
                if (!spelling_in_main_file(sm_, EC->getLocation())) continue;
                emit_named(EC, EC, KIND_VALUE);
            }
        }
    }

    void handle_record(const clang::CXXRecordDecl* R) {
        // Skip anonymous inline structs/unions used only to introduce fields.
        if (R->isAnonymousStructOrUnion()) return;

        const clang::Decl* pick = pick_main_file_decl(R, sm_);
        if (pick == R) {
            emit_named(R, R, KIND_CLASS);
        }
        // Walk the body only for the actual definition. Explicit template
        // specializations that also carry a body are their own definitions
        // and are still visited when we reach them via the containing decl
        // context.
        if (R->isThisDeclarationADefinition()) {
            walk_record_body(R);
        }
    }

    void walk_record_body(const clang::CXXRecordDecl* R) {
        for (const clang::Decl* Child : R->decls()) {
            if (Child->isImplicit()) continue;
            // Explicit access specifiers, base clauses, and friend declarations
            // are not emitted as symbols; walking them through `visit` would
            // fall through classify() safely, but skipping saves work.
            if (llvm::isa<clang::AccessSpecDecl>(Child)) continue;
            if (llvm::isa<clang::FriendDecl>(Child)) continue;

            // Nested tags (class inside class), nested typedefs/aliases,
            // static/instance methods, static data members, member function
            // templates, member class templates, and nested enum decls all
            // dispatch through the regular visitor.
            visit(Child);
        }
    }

    void emit_named(const clang::Decl* anchor,
                    const clang::NamedDecl* named,
                    const char* kind,
                    std::string signature = {}) {
        if (kind == nullptr) return;
        // Filter: only decls whose spelling location lives inside the main
        // file survive.
        if (!spelling_in_main_file(sm_, named->getLocation())) return;

        // Redeclaration selection: for classes, functions, variables we only
        // emit for the chosen decl. Everything else (namespaces, enums,
        // methods, etc.) is handled by its containing walker.
        if (llvm::isa<clang::CXXRecordDecl>(named) ||
            llvm::isa<clang::FunctionDecl>(named) ||
            (llvm::isa<clang::VarDecl>(named) && !llvm::isa<clang::EnumConstantDecl>(named))) {
            if (pick_main_file_decl(named, sm_) != named) return;
        }

        std::string name = simple_name(named);
        if (name.empty()) {
            // Emit a location-based fallback so anonymous tagged decls (e.g.
            // `class {} obj;`) still land in the search index. Anonymous
            // namespaces are handled by `getQualifiedNameAsString` producing
            // `(anonymous namespace)::…`, so they never hit this branch.
            auto [line, col] = spelling_line_col(sm_, named->getLocation());
            if (line == 0) return;
            name = "(anonymous@" + std::to_string(line) + ":" + std::to_string(col) + ")";
        }

        RawSymbol s;
        s.name = name;
        s.kind = kind;
        s.qualified_name = qualified_name(named);
        s.signature = std::move(signature);

        auto [sl, sc] = spelling_line_col(sm_, anchor->getBeginLoc());
        if (sl == 0) {
            dropped_location_ = true;
            return;
        }
        s.start_line = sl;
        s.start_column = sc;

        clang::SourceLocation end = end_of_token(sm_, langopts_, anchor->getEndLoc());
        if (spelling_in_main_file(sm_, end)) {
            auto [el, ec] = spelling_line_col(sm_, end);
            if (el >= sl && !(el == sl && ec < sc)) {
                s.end_line = el;
                s.end_column = ec;
                s.has_end = true;
            } else {
                dropped_location_ = true;
            }
        } else {
            dropped_location_ = true;
        }

        symbols_.push_back(std::move(s));
    }

    clang::ASTContext& ctx_;
    const clang::SourceManager& sm_;
    const clang::LangOptions& langopts_;
    std::string target_path_;
    std::vector<RawSymbol> symbols_;
    bool dropped_location_ = false;
};

// ─── Assembly ───────────────────────────────────────────────────────────────

Symbol to_wire_symbol(const RawSymbol& raw, const std::string& target_path) {
    Symbol s;
    s.name = raw.name;
    s.kind = raw.kind;
    if (!raw.qualified_name.empty() && raw.qualified_name != raw.name) {
        s.qualified_name = raw.qualified_name;
    }
    s.search_names.push_back(raw.name);
    if (s.qualified_name.has_value()) {
        s.search_names.push_back(*s.qualified_name);
    }
    if (!raw.signature.empty()) {
        s.signature = raw.signature;
    }
    Location loc;
    loc.path = target_path;
    loc.start.line = raw.start_line;
    loc.start.column = raw.start_column;
    if (raw.has_end) {
        Position end;
        end.line = raw.end_line;
        end.column = raw.end_column;
        loc.end = end;
    }
    s.location = loc;
    return s;
}

// ─── Frontend driver ────────────────────────────────────────────────────────

/// Frontend action that installs a `SymbolCollector` after the AST is built.
/// We use `ASTFrontendAction` so we get a fully typed `ASTContext` — the
/// preprocess-only path in `dependencies.cpp` is not enough here.
class SymbolAction : public clang::ASTFrontendAction {
   public:
    SymbolAction(std::string target_path, SymbolOutcome* out)
        : target_path_(std::move(target_path)), out_(out) {}

   protected:
    std::unique_ptr<clang::ASTConsumer> CreateASTConsumer(clang::CompilerInstance& CI,
                                                          llvm::StringRef /*file*/) override {
        (void)CI;
        return std::make_unique<Consumer>(target_path_, out_);
    }

   private:
    class Consumer : public clang::ASTConsumer {
       public:
        Consumer(std::string target_path, SymbolOutcome* out)
            : target_path_(std::move(target_path)), out_(out) {}
        void HandleTranslationUnit(clang::ASTContext& ctx) override {
            SymbolCollector collector(ctx, target_path_);
            collector.run();

            std::vector<RawSymbol>& raws = collector.symbols();
            std::sort(raws.begin(), raws.end(), [](const RawSymbol& a, const RawSymbol& b) {
                if (a.start_line != b.start_line) return a.start_line < b.start_line;
                return a.start_column < b.start_column;
            });

            SymbolAnalysis& sa = out_->analysis;
            sa.state = AnalysisState::Complete;
            sa.symbols.reserve(raws.size());
            for (const auto& r : raws) {
                sa.symbols.push_back(to_wire_symbol(r, target_path_));
            }
            if (collector.has_dropped_location()) {
                sa.state = AnalysisState::Partial;
            }
        }

       private:
        std::string target_path_;
        SymbolOutcome* out_;
    };

    std::string target_path_;
    SymbolOutcome* out_;
};

}  // namespace

SymbolOutcome analyzeSymbols(clang::ASTContext& context,
                             const fs::path& /*managedSource*/,
                             const std::string& target_path) {
    SymbolOutcome outcome;
    SymbolCollector collector(context, target_path);
    collector.run();

    std::vector<RawSymbol>& raws = collector.symbols();
    std::sort(raws.begin(), raws.end(), [](const RawSymbol& a, const RawSymbol& b) {
        if (a.start_line != b.start_line) return a.start_line < b.start_line;
        return a.start_column < b.start_column;
    });

    outcome.analysis.state = AnalysisState::Complete;
    outcome.analysis.symbols.reserve(raws.size());
    for (const auto& r : raws) {
        outcome.analysis.symbols.push_back(to_wire_symbol(r, target_path));
    }
    if (collector.has_dropped_location()) {
        outcome.analysis.state = AnalysisState::Partial;
    }
    return outcome;
}

SymbolOutcome analyzeSymbolsForTarget(const CompileProfile& profile,
                                      const fs::path& source_file,
                                      const std::string& target_path) {
    SymbolOutcome outcome;
    outcome.analysis.state = AnalysisState::Complete;

    Target tgt;
    tgt.source_file = source_file;
    std::vector<std::string> argv;
    argv.emplace_back("cpp-analyzer");
    for (auto& arg : buildClangArguments(profile, tgt)) {
        argv.push_back(std::move(arg));
    }

    llvm::IntrusiveRefCntPtr<llvm::vfs::FileSystem> vfs = llvm::vfs::getRealFileSystem();
    llvm::IntrusiveRefCntPtr<clang::FileManager> file_mgr(
        new clang::FileManager(clang::FileSystemOptions{}, vfs));

    auto action = std::make_unique<SymbolAction>(target_path, &outcome);
    clang::tooling::ToolInvocation invocation(std::move(argv), std::move(action),
                                              file_mgr.get());
    // Redirect diagnostics into a no-op consumer for the same reason as the
    // dependency analyzer: partial parses are expected inputs — enough
    // to inform the state but not to pollute stdout with duplicated noise.
    invocation.setDiagnosticConsumer(new clang::IgnoringDiagConsumer());

    bool ok = invocation.run();
    if (!ok && outcome.analysis.symbols.empty()) {
        outcome.analysis.state = AnalysisState::Failed;
    } else if (!ok) {
        // Ran, produced some symbols, but the driver reported errors — treat
        // as partial rather than complete so callers know coverage may be
        // incomplete.
        outcome.analysis.state = AnalysisState::Partial;
    }
    return outcome;
}

}  // namespace ce_cpp
