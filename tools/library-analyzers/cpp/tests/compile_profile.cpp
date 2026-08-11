// Unit tests for the ce-cpp compile profile loader (spec §6.7; plan 046
// Task 1).
//
// Each test writes an isolated on-disk fixture under a fresh temporary
// directory: a repository root, an optional `libraries/cpp` include tree, and
// a `tools/library-analyzers/cpp/compile-profile.toml`. The loader is then
// invoked with the temporary repository root and either returns a populated
// `CompileProfile` or throws `CompileProfileError`.
//
// Environment independence is asserted by setting `CXX`, `CXXFLAGS`, and
// `CPATH` to bogus values around a load and confirming the resulting argv is
// unchanged. Determinism is asserted by running the same load twice and
// comparing byte-for-byte.

#include "compile_profile.hpp"

#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <random>
#include <string>
#include <sys/stat.h>
#include <unistd.h>

namespace fs = std::filesystem;

namespace {

int failures = 0;

#define CE_EXPECT_TRUE(expr)                                                                   \
    do {                                                                                       \
        if (!(expr)) {                                                                         \
            std::cerr << __FILE__ << ":" << __LINE__ << ": expectation failed: " << #expr      \
                      << std::endl;                                                            \
            ++failures;                                                                        \
        }                                                                                      \
    } while (0)

#define CE_EXPECT_EQ(a, b)                                                                     \
    do {                                                                                       \
        auto _a = (a);                                                                         \
        auto _b = (b);                                                                         \
        if (!(_a == _b)) {                                                                     \
            std::cerr << __FILE__ << ":" << __LINE__ << ": expected equality: " << #a << " == "\
                      << #b << std::endl;                                                      \
            ++failures;                                                                        \
        }                                                                                      \
    } while (0)

#define CE_EXPECT_THROWS(expr, exc_type)                                                       \
    do {                                                                                       \
        bool _threw = false;                                                                   \
        try {                                                                                  \
            (void)(expr);                                                                      \
        } catch (const exc_type&) {                                                            \
            _threw = true;                                                                     \
        } catch (const std::exception& _ex) {                                                  \
            std::cerr << __FILE__ << ":" << __LINE__ << ": wrong exception type: "             \
                      << _ex.what() << std::endl;                                              \
            ++failures;                                                                        \
        }                                                                                      \
        if (!_threw) {                                                                         \
            std::cerr << __FILE__ << ":" << __LINE__ << ": expected " << #exc_type             \
                      << " from " << #expr << std::endl;                                       \
            ++failures;                                                                        \
        }                                                                                      \
    } while (0)

/// Create a unique temporary directory under the system temp root. Uses
/// `mkdtemp` for atomic creation with mode 0700.
fs::path make_temp_dir() {
    fs::path base = fs::temp_directory_path() / "ce-cpp-compile-profile-XXXXXX";
    std::string tmpl = base.string();
    std::vector<char> buf(tmpl.begin(), tmpl.end());
    buf.push_back('\0');
    if (mkdtemp(buf.data()) == nullptr) {
        std::cerr << "mkdtemp failed: " << std::strerror(errno) << std::endl;
        std::abort();
    }
    return fs::path(buf.data());
}

/// Write `contents` to `path`, creating parent directories as needed.
void write_file(const fs::path& path, const std::string& contents) {
    fs::create_directories(path.parent_path());
    std::ofstream f(path);
    if (!f.is_open()) {
        std::cerr << "failed to open " << path << " for write" << std::endl;
        std::abort();
    }
    f << contents;
}

struct TempRepo {
    fs::path root;

    explicit TempRepo() : root(make_temp_dir()) {}
    ~TempRepo() {
        std::error_code ec;
        fs::remove_all(root, ec);
    }
    TempRepo(const TempRepo&) = delete;
    TempRepo& operator=(const TempRepo&) = delete;

    void write_profile(const std::string& body) const {
        write_file(root / "tools/library-analyzers/cpp/compile-profile.toml", body);
    }
};

// ─── Tests ──────────────────────────────────────────────────────────────────

void test_loads_all_three_fields() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = [\"CE_LIB=1\", \"NDEBUG\"]\n"
        "include_roots = [\"libraries/cpp\"]\n");
    auto profile = ce_cpp::loadCompileProfile(repo.root);
    CE_EXPECT_EQ(profile.cxx_standard, std::string("c++20"));
    CE_EXPECT_EQ(profile.defines.size(), size_t{2});
    CE_EXPECT_EQ(profile.defines[0], std::string("CE_LIB=1"));
    CE_EXPECT_EQ(profile.defines[1], std::string("NDEBUG"));
    CE_EXPECT_EQ(profile.include_roots.size(), size_t{1});
    CE_EXPECT_EQ(profile.include_roots[0], fs::canonical(repo.root / "libraries/cpp"));
}

void test_rejects_duplicate_key() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "cxx_standard = \"c++23\"\n"
        "defines = []\n"
        "include_roots = [\"libraries/cpp\"]\n");
    CE_EXPECT_THROWS(ce_cpp::loadCompileProfile(repo.root), ce_cpp::CompileProfileError);
}

void test_rejects_missing_key() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "include_roots = [\"libraries/cpp\"]\n");
    CE_EXPECT_THROWS(ce_cpp::loadCompileProfile(repo.root), ce_cpp::CompileProfileError);
}

void test_rejects_unknown_key() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = []\n"
        "include_roots = [\"libraries/cpp\"]\n"
        "extra_field = \"nope\"\n");
    CE_EXPECT_THROWS(ce_cpp::loadCompileProfile(repo.root), ce_cpp::CompileProfileError);
}

void test_rejects_absolute_include_root() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = []\n"
        "include_roots = [\"/etc\"]\n");
    CE_EXPECT_THROWS(ce_cpp::loadCompileProfile(repo.root), ce_cpp::CompileProfileError);
}

void test_rejects_repository_escape() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    // `..` component would escape the repository root even though the string
    // is relative.
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = []\n"
        "include_roots = [\"../outside\"]\n");
    CE_EXPECT_THROWS(ce_cpp::loadCompileProfile(repo.root), ce_cpp::CompileProfileError);
}

void test_symlink_include_root_resolves_inside_repo() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp/graph");
    // A symlink whose *target* points to another directory inside the repo
    // resolves to that directory and is accepted.
    fs::path link = repo.root / "libraries/cpp/graph-alias";
    std::error_code ec;
    fs::create_directory_symlink("graph", link, ec);
    if (ec) {
        // Filesystem does not support symlinks (e.g., some CI sandboxes).
        std::cerr << "skipping symlink test: " << ec.message() << std::endl;
        return;
    }
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = []\n"
        "include_roots = [\"libraries/cpp/graph-alias\"]\n");
    auto profile = ce_cpp::loadCompileProfile(repo.root);
    CE_EXPECT_EQ(profile.include_roots.size(), size_t{1});
    CE_EXPECT_EQ(profile.include_roots[0], fs::canonical(repo.root / "libraries/cpp/graph"));
}

void test_symlink_that_escapes_repository_is_rejected() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    fs::path outside = make_temp_dir();
    fs::path link = repo.root / "libraries/cpp/outside-link";
    std::error_code ec;
    fs::create_directory_symlink(outside, link, ec);
    if (ec) {
        std::cerr << "skipping symlink-escape test: " << ec.message() << std::endl;
        std::error_code rm_ec;
        fs::remove_all(outside, rm_ec);
        return;
    }
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = []\n"
        "include_roots = [\"libraries/cpp/outside-link\"]\n");
    CE_EXPECT_THROWS(ce_cpp::loadCompileProfile(repo.root), ce_cpp::CompileProfileError);
    std::error_code rm_ec;
    fs::remove_all(outside, rm_ec);
}

void test_environment_independence() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = [\"CE_LIB=1\"]\n"
        "include_roots = [\"libraries/cpp\"]\n");
    auto baseline = ce_cpp::loadCompileProfile(repo.root);
    ce_cpp::Target target;
    target.source_file = repo.root / "libraries/cpp/a.cpp";
    auto baseline_argv = ce_cpp::buildClangArguments(baseline, target);

    // Poison the environment with settings a naive loader might pick up.
    ::setenv("CXX", "/opt/definitely-not-clang", 1);
    ::setenv("CXXFLAGS", "-std=c++03 -DBOGUS=1", 1);
    ::setenv("CPATH", "/opt/should-not-appear", 1);
    ::setenv("CPLUS_INCLUDE_PATH", "/opt/should-not-appear", 1);

    auto reload = ce_cpp::loadCompileProfile(repo.root);
    auto reload_argv = ce_cpp::buildClangArguments(reload, target);
    CE_EXPECT_EQ(reload_argv, baseline_argv);

    ::unsetenv("CXX");
    ::unsetenv("CXXFLAGS");
    ::unsetenv("CPATH");
    ::unsetenv("CPLUS_INCLUDE_PATH");
}

void test_deterministic_argv_order() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    fs::create_directories(repo.root / "libraries/cpp/std");
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = [\"A=1\", \"B=2\", \"C=3\"]\n"
        "include_roots = [\"libraries/cpp\", \"libraries/cpp/std\"]\n");
    auto profile = ce_cpp::loadCompileProfile(repo.root);
    ce_cpp::Target target;
    target.source_file = repo.root / "libraries/cpp/a.cpp";
    auto one = ce_cpp::buildClangArguments(profile, target);
    auto two = ce_cpp::buildClangArguments(profile, target);
    CE_EXPECT_EQ(one, two);

    // Order: `-x c++`, `-std=c++20`, three `-D`s in declared order, two
    // `-I`s in declared order, and finally the source file.
    CE_EXPECT_EQ(one.size(), size_t{9});
    CE_EXPECT_EQ(one[0], std::string("-x"));
    CE_EXPECT_EQ(one[1], std::string("c++"));
    CE_EXPECT_EQ(one[2], std::string("-std=c++20"));
    CE_EXPECT_EQ(one[3], std::string("-DA=1"));
    CE_EXPECT_EQ(one[4], std::string("-DB=2"));
    CE_EXPECT_EQ(one[5], std::string("-DC=3"));
    const auto include_root_one = fs::canonical(repo.root / "libraries/cpp");
    const auto include_root_two = fs::canonical(repo.root / "libraries/cpp/std");
    CE_EXPECT_EQ(one[6], std::string("-I") + include_root_one.string());
    CE_EXPECT_EQ(one[7], std::string("-I") + include_root_two.string());
    CE_EXPECT_EQ(one[8], target.source_file.string());
}

void test_rejects_non_string_standard() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    repo.write_profile(
        "cxx_standard = 20\n"
        "defines = []\n"
        "include_roots = [\"libraries/cpp\"]\n");
    CE_EXPECT_THROWS(ce_cpp::loadCompileProfile(repo.root), ce_cpp::CompileProfileError);
}

void test_rejects_missing_include_root_directory() {
    TempRepo repo;
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = []\n"
        "include_roots = [\"libraries/cpp\"]\n");
    // The `libraries/cpp` directory was never created.
    CE_EXPECT_THROWS(ce_cpp::loadCompileProfile(repo.root), ce_cpp::CompileProfileError);
}

void test_rejects_source_file_outside_repository() {
    TempRepo repo;
    fs::create_directories(repo.root / "libraries/cpp");
    repo.write_profile(
        "cxx_standard = \"c++20\"\n"
        "defines = []\n"
        "include_roots = [\"libraries/cpp\"]\n");
    auto profile = ce_cpp::loadCompileProfile(repo.root);
    ce_cpp::Target target;
    target.source_file = fs::path("/definitely/outside/a.cpp");
    CE_EXPECT_THROWS(ce_cpp::buildClangArguments(profile, target), ce_cpp::CompileProfileError);
}

}  // namespace

int main() {
    test_loads_all_three_fields();
    test_rejects_duplicate_key();
    test_rejects_missing_key();
    test_rejects_unknown_key();
    test_rejects_absolute_include_root();
    test_rejects_repository_escape();
    test_symlink_include_root_resolves_inside_repo();
    test_symlink_that_escapes_repository_is_rejected();
    test_environment_independence();
    test_deterministic_argv_order();
    test_rejects_non_string_standard();
    test_rejects_missing_include_root_directory();
    test_rejects_source_file_outside_repository();

    if (failures > 0) {
        std::cerr << failures << " compile-profile assertions failed" << std::endl;
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
