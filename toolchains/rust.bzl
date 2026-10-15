"""Rust toolchains built from the archives pinned in rust_release.bzl."""

load("@prelude//decls:toolchains_common.bzl", "toolchains_common")
load("@prelude//python_bootstrap:python_bootstrap.bzl", "PythonBootstrapToolchainInfo")
load("@prelude//rust:rust_toolchain.bzl", "PanicRuntime", "RustToolchainInfo")
load(":rust_release.bzl", "RUSTFMT_HOSTS", "RUST_HOSTS")

# Host tables of rust_release.bzl that `rust_release` accepts.
_HOST_TABLES = {
    "release": RUST_HOSTS,
    "rustfmt-nightly": RUSTFMT_HOSTS,
}

RustReleaseInfo = provider(fields = {
    # The component archives in one directory, laid out like a rustup
    # toolchain.
    "sysroot": provider_field(Artifact),
    "executable_suffix": provider_field(str),
    "host_triple": provider_field(str),
})

def _select_host(hosts: dict, field: typing.Callable) -> Select:
    """Selects `field(host)` for the configured host."""
    return select({
        "prelude//os:" + os_name: select({
            "prelude//cpu:" + cpu: field(host)
            for cpu, host in cpus.items()
        })
        for os_name, cpus in hosts.items()
    })

def _rust_release_archives(name: str, hosts: dict) -> list[str]:
    """Declares an `http_file` per component and returns their labels."""
    components = hosts.values()[0].values()[0]["archives"].keys()
    labels = []
    for component in components:
        archive = "{}-{}-archive".format(name, component)
        native.http_file(
            name = archive,
            urls = [_select_host(hosts, lambda host: host["archives"][component]["url"])],
            sha256 = _select_host(hosts, lambda host: host["archives"][component]["sha256"]),
            out = "{}.tar.xz".format(component),
        )
        labels.append(":" + archive)
    return labels

def _rust_release_impl(ctx: AnalysisContext) -> list[Provider]:
    sysroot = ctx.actions.declare_output("sysroot", dir = True)
    ctx.actions.run(
        cmd_args(
            ctx.attrs._python_bootstrap_toolchain[PythonBootstrapToolchainInfo].interpreter,
            ctx.attrs._assembler,
            sysroot.as_output(),
            [archive[DefaultInfo].default_outputs[0] for archive in ctx.attrs.archives],
        ),
        category = "rust_sysroot",
    )
    return [
        DefaultInfo(default_output = sysroot),
        RustReleaseInfo(
            sysroot = sysroot,
            executable_suffix = ctx.attrs.executable_suffix,
            host_triple = ctx.attrs.host_triple,
        ),
    ]

# The host toolchain merged from the release archives. Toolchains reach it
# through `attrs.exec_dep`, so it is configured once for the execution
# platform and its selects describe the host.
_rust_release = rule(
    impl = _rust_release_impl,
    attrs = {
        "archives": attrs.list(attrs.dep()),
        "executable_suffix": attrs.string(),
        "host_triple": attrs.string(),
        "_assembler": attrs.source(default = "toolchains//:assemble_rust_sysroot.py"),
        "_python_bootstrap_toolchain": toolchains_common.python_bootstrap(),
    },
)

def rust_release(name: str, hosts: str, visibility: list[str] = []):
    """Assembles the archives of one host table of rust_release.bzl:
    `release` or `rustfmt-nightly`."""
    table = _HOST_TABLES[hosts]
    _rust_release(
        name = name,
        archives = _rust_release_archives(name, table),
        executable_suffix = select({
            "DEFAULT": "",
            "prelude//os:windows": ".exe",
        }),
        host_triple = _select_host(table, lambda host: host["triple"]),
        visibility = visibility,
    )

def _rust_release_toolchain_impl(ctx: AnalysisContext) -> list[Provider]:
    release = ctx.attrs.release[RustReleaseInfo]

    def tool(name):
        return RunInfo(args = cmd_args(release.sysroot, format = "{}/bin/" + name + release.executable_suffix))

    return [
        DefaultInfo(),
        RustToolchainInfo(
            clippy_driver = tool("clippy-driver"),
            compiler = tool("rustc"),
            default_edition = ctx.attrs.default_edition,
            panic_runtime = PanicRuntime("unwind"),
            rustc_flags = ctx.attrs.rustc_flags,
            rustc_target_triple = release.host_triple,
            rustdoc = tool("rustdoc"),
            rustdoc_flags = ctx.attrs.rustdoc_flags,
        ),
    ]

# Buck2 here builds only for the host it runs on, so the execution
# platform's toolchain and standard library also serve the target.
rust_release_toolchain = rule(
    impl = _rust_release_toolchain_impl,
    attrs = {
        "default_edition": attrs.option(attrs.string(), default = None),
        "release": attrs.exec_dep(providers = [RustReleaseInfo]),
        "rustc_flags": attrs.list(attrs.string(), default = []),
        "rustdoc_flags": attrs.list(attrs.string(), default = []),
    },
    is_toolchain_rule = True,
)

def _release_tool_impl(ctx: AnalysisContext) -> list[Provider]:
    release = ctx.attrs.release[RustReleaseInfo]
    return [
        DefaultInfo(),
        RunInfo(args = cmd_args(release.sysroot, format = "{}/bin/" + ctx.attrs.tool + release.executable_suffix)),
    ]

# An executable of the Rust release, for example rustfmt, to `buck2 run`.
release_tool = rule(
    impl = _release_tool_impl,
    attrs = {
        "release": attrs.exec_dep(providers = [RustReleaseInfo]),
        "tool": attrs.string(),
    },
)
