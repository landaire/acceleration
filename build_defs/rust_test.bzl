"""`rust_test` with Buck2's internal test runner.

Each libtest case is listed and run in its own process, and Buck2 reports a
result and duration for each case. This requires buck2 2026-10-01 or later;
toolchains/buck2_release.json pins such a release.
"""

load("@prelude//rules.bzl", "clone_rule")
load("@prelude//rust:rust_binary.bzl", "rust_test_impl")

_LIBTEST_STATUS = {
    "failed": "FAIL",
    "ignored": "SKIP",
    "passed": "PASS",
}

def _parse_test_listing(listing: str) -> list[dict[str, str]]:
    """Parses `--list --format terse` output: one `<name>: test` line per case."""
    cases = []
    for line in listing.splitlines():
        if line.endswith(": test"):
            name = line.removesuffix(": test")
            cases.append({"filter": name, "name": name})
    return cases

def _parse_summary(line: str) -> (dict[str, int], float | None):
    """Parses `test result: ok. 1 passed; 0 failed; ...; finished in 0.01s`."""
    counts = {}
    duration = None
    for field in line.partition(". ")[2].split("; "):
        if field.startswith("finished in ") and field.endswith("s"):
            duration = float(field.removeprefix("finished in ").removesuffix("s"))
        else:
            count, _, kind = field.partition(" ")
            if count.isdigit():
                counts[kind] = int(count)
    return counts, duration

def _parse_test_result(stdout: str, stderr: str, exit_code: int) -> list[dict[str, typing.Any]]:
    """Reports the single case selected by `--exact <name>`.

    Returns no entries when libtest did not print both the case line and the
    summary (for example after a crash); Buck2 then reports the case from
    the exit code.
    """
    name = None
    counts = None
    duration = None
    for line in stdout.splitlines():
        if name == None and line.startswith("test ") and " ... " in line:
            # `#[should_panic]` cases print as `test <name> - should panic ... `.
            name = line.removeprefix("test ").partition(" ... ")[0].removesuffix(" - should panic")
        elif line.startswith("test result: "):
            counts, duration = _parse_summary(line)
    if name == None or counts == None:
        return []

    status = None
    for kind, value in _LIBTEST_STATUS.items():
        if counts.get(kind) == 1:
            status = value
    if status == None:
        return []

    # A case that passes inside libtest still fails if the process does
    # not exit cleanly, for example when a destructor aborts.
    if status != "FAIL" and exit_code != 0:
        status = "FAIL"

    # libtest prints a failing case's captured output; passing cases have
    # none worth showing.
    details = None
    if status == "FAIL":
        details = "---- STDOUT ----\n{}\n---- STDERR ----\n{}\n".format(stdout, stderr)

    return [{
        "details": details,
        "duration": duration,
        "name": name,
        "status": status,
    }]

def _rust_test_impl(ctx: AnalysisContext) -> list[Provider]:
    providers = rust_test_impl(ctx)

    # `framework = False` binaries do not implement the libtest interface.
    if not ctx.attrs.framework:
        return providers

    result = []
    for provider in providers:
        if isinstance(provider, ExternalRunnerTestInfo):
            result.append(InternalRunnerTestInfo(
                type = provider.test_type,
                listing_command = provider.command + ["--list", "--format", "terse"],
                command = provider.command + ["--color", "never", "--exact"],
                env = provider.env,
                labels = provider.labels,
                contacts = provider.contacts,
                run_from_project_root = provider.run_from_project_root,
                use_project_relative_paths = provider.use_project_relative_paths,
                default_executor = provider.default_executor,
                executor_overrides = provider.executor_overrides,
                parse_test_listing = _parse_test_listing,
                parse_test_result = _parse_test_result,
            ))
        else:
            result.append(provider)
    return result

rust_test = clone_rule("rust_test", impl_override = _rust_test_impl)
