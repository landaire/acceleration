"""The rules that the generated third-party/rust/BUCK calls.

They are the prelude's reindeer macros with two changes: a crate, and the C
libraries of its fixups, are limited to the hosts whose reindeer platform
uses the crate, and crate archives define no test.
"""

load("@prelude//http_archive:http_archive.bzl", "http_archive_impl")
load("@prelude//rules.bzl", "clone_rule")
load("@prelude//rust:cargo_package.bzl", "apply_platform_attrs", _cargo = "cargo")

# Reindeer platforms in reindeer.toml and the host OS of each.
_PLATFORM_OS = {
    "linux-x86_64": "prelude//os:linux",
    "macos-arm64": "prelude//os:macos",
    "windows-msvc": "prelude//os:windows",
}

def _crate_archive_impl(ctx: AnalysisContext) -> list[Provider]:
    # The prelude's http_archive also returns a test that only materializes
    # the archive, so `buck2 test //...` would report one per crate.
    return [p for p in http_archive_impl(ctx) if not isinstance(p, ExternalRunnerTestInfo)]

# The downloaded source of a crates.io crate.
crate_archive = clone_rule("http_archive", impl_override = _crate_archive_impl)

def _platform_gated(name: str, kwargs: dict) -> dict:
    """Limits a rule to the hosts in its `platform` attribute.

    reindeer.toml sets `platform_compatibility_on_all_targets`, so the
    attribute names every platform that uses the crate.
    """
    if "compatible_with" in kwargs:
        fail("{}: a fixup sets compatible_with; the reindeer platforms limit crates".format(name))
    platforms = kwargs.get("platform", {})
    for platform in platforms:
        if platform not in _PLATFORM_OS:
            fail("reindeer platform `{}` has no host OS in build_defs/third_party.bzl".format(platform))
    if len(platforms) == len(_PLATFORM_OS):
        return kwargs
    return kwargs | {"compatible_with": [_PLATFORM_OS[platform] for platform in sorted(platforms)]}

def _rust_library(name: str, **kwargs):
    # Proc macros are not limited: they run on the execution platform, and
    # the prelude configures them where no target platform applies.
    if not kwargs.get("proc_macro"):
        kwargs = _platform_gated(name, kwargs)
    _cargo.rust_library(name = name, **kwargs)

def _rust_binary(name: str, crate: str | None = None, **kwargs):
    # Build scripts run on the execution platform, which the prelude selects
    # per target platform; only the crates themselves are limited.
    if crate != "build_script_build":
        kwargs = _platform_gated(name, kwargs)
    _cargo.rust_binary(name = name, crate = crate, **kwargs)

def _cxx_library(name: str, **kwargs):
    kwargs = _platform_gated(name, kwargs)
    platform = kwargs.pop("platform", {})
    native.cxx_library(name = name, **apply_platform_attrs(platform, kwargs))

def _prebuilt_cxx_library(name: str, **kwargs):
    kwargs = _platform_gated(name, kwargs)
    platform = kwargs.pop("platform", {})
    native.prebuilt_cxx_library(name = name, **apply_platform_attrs(platform, kwargs))

cargo = struct(
    cxx_library = _cxx_library,
    http_archive = crate_archive,
    prebuilt_cxx_library = _prebuilt_cxx_library,
    rust_binary = _rust_binary,
    rust_library = _rust_library,
)
