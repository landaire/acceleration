"""An execution platform that consults a remote action cache.

Actions still run locally. When [buck2_re_client] names a cache, Buck2
first looks up each action's digest there and uploads the result after
running it. CI selects this platform with ci/configure-action-cache.sh and
points [buck2_re_client] at the shared cache of landaire/build-cache when
the job can reach it; otherwise the platform builds without a cache.
"""

def _cached_execution_platform_impl(ctx: AnalysisContext) -> list[Provider]:
    constraints = dict()
    constraints.update(ctx.attrs.cpu_configuration[ConfigurationInfo].constraints)
    constraints.update(ctx.attrs.os_configuration[ConfigurationInfo].constraints)
    configuration = ConfigurationInfo(constraints = constraints, values = {})

    # Action keys cover the C compiler wrappers in toolchains/ but not the
    # compiler, linker and MSVC that they run from the host. The platform
    # properties are part of every action key, so naming the host tools keeps
    # hosts with different tools from sharing entries.
    if ctx.attrs.remote_cache_enabled and not ctx.attrs.host_tools:
        fail("[buck2_re_client] names a cache, but [action_cache] host_tools is not set; see ci/configure-action-cache.sh")
    properties = {"host-tools": ctx.attrs.host_tools} if ctx.attrs.host_tools else None

    platform = ExecutionPlatformInfo(
        label = ctx.label.raw_target(),
        configuration = configuration,
        executor_config = CommandExecutorConfig(
            local_enabled = True,
            # Without remote execution Buck2 runs every action locally and
            # consults only the action cache. Remote execution would select
            # the hybrid executor, which prepares every action for the
            # remote side and uses far more daemon memory.
            remote_enabled = False,
            remote_cache_enabled = ctx.attrs.remote_cache_enabled,
            allow_cache_uploads = ctx.attrs.remote_cache_enabled,
            remote_execution_properties = properties,
            use_windows_path_separators = ctx.attrs.use_windows_path_separators,
        ),
    )
    return [
        DefaultInfo(),
        platform,
        PlatformInfo(label = str(ctx.label.raw_target()), configuration = configuration),
        ExecutionPlatformRegistrationInfo(platforms = [platform]),
    ]

cached_execution_platform = rule(
    impl = _cached_execution_platform_impl,
    attrs = {
        "cpu_configuration": attrs.dep(providers = [ConfigurationInfo]),
        "host_tools": attrs.option(attrs.string(), default = None),
        "os_configuration": attrs.dep(providers = [ConfigurationInfo]),
        "remote_cache_enabled": attrs.bool(default = False),
        "use_windows_path_separators": attrs.bool(default = False),
    },
)

def remote_cache_configured() -> bool:
    """Whether this checkout names an action cache.

    Enabling the cache without one is fatal, not a fallback to local: every
    action fails with "No engine address". A CI job that could not reach the
    cache has no [buck2_re_client] section and builds without it.
    """
    return read_root_config("buck2_re_client", "engine_address") != None

def host_tools() -> str | None:
    """The name of the host's C tools, which CI sets to the runner image."""
    return read_root_config("action_cache", "host_tools")
