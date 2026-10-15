"""C/C++ tools for the host toolchain."""

load("@prelude//cxx:cxx_toolchain_types.bzl", "LinkerType")
load("@prelude//toolchains:cxx.bzl", "CxxToolsInfo")

def _path_cxx_tools_impl(ctx: AnalysisContext) -> list[Provider]:
    return [
        DefaultInfo(),
        CxxToolsInfo(
            compiler = ctx.attrs.cc,
            compiler_type = ctx.attrs.compiler_type,
            cxx_compiler = ctx.attrs.cxx,
            asm_compiler = ctx.attrs.cc,
            asm_compiler_type = ctx.attrs.compiler_type,
            rc_compiler = None,
            cvtres_compiler = None,
            archiver = ctx.attrs.ar,
            archiver_type = "gnu",
            linker = ctx.attrs.cxx,
            linker_type = LinkerType(ctx.attrs.linker_type),
        ),
    ]

# The C and C++ compiler drivers on PATH, through wrapper scripts: the
# prelude runs build-script compilers with exec, which does not search PATH.
path_cxx_tools = rule(
    impl = _path_cxx_tools_impl,
    attrs = {
        "ar": attrs.source(),
        "cc": attrs.source(),
        # gcc for `cc`, which can be GCC or Clang: Clang accepts GCC's flags,
        # and GCC rejects Clang-only ones.
        "compiler_type": attrs.string(),
        "cxx": attrs.source(),
        "linker_type": attrs.string(),
    },
)
