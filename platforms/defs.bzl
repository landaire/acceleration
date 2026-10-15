"""An execution platform that consults a remote action cache.

Actions still run locally. With `remote_cache_enabled`, Buck2 first looks
up each action's digest in the cache that [buck2_re_client] names, and
uploads the result after running it. CI points that section at a local
bazel-remote server whose storage persists between runs (see
ci/start-action-cache.sh); without it, builds use the prelude's local-only
platform.
"""

def _cached_execution_platform_impl(ctx: AnalysisContext) -> list[Provider]:
    constraints = dict()
    constraints.update(ctx.attrs.cpu_configuration[ConfigurationInfo].constraints)
    constraints.update(ctx.attrs.os_configuration[ConfigurationInfo].constraints)
    configuration = ConfigurationInfo(constraints = constraints, values = {})

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
            remote_cache_enabled = True,
            allow_cache_uploads = True,
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
        "os_configuration": attrs.dep(providers = [ConfigurationInfo]),
        "use_windows_path_separators": attrs.bool(default = False),
    },
)
