"""Smoke tests for the xex2 Python bindings.

Exercise the diplomat-generated C++ layer via nanobind end-to-end. No
network, no deterministic fixtures — they rely on an XEX under
`xex_files/` at the workspace root. When the fixture is absent the
tests are skipped rather than failed so fresh clones still run green.
"""

from __future__ import annotations

import pathlib

import pytest

import xex2


WORKSPACE_ROOT = pathlib.Path(__file__).resolve().parents[3]
SAMPLE_XEX = WORKSPACE_ROOT / "xex_files" / "afplayer.xex"


# ── Parse error handling ─────────────────────────────────────────────────


def test_parse_empty_raises():
	with pytest.raises(RuntimeError):
		xex2.Xex2.parse(b"")


def test_parse_garbage_raises():
	with pytest.raises(RuntimeError):
		xex2.Xex2.parse(b"not an xex" * 64)


# ── Real-fixture tests ───────────────────────────────────────────────────


@pytest.fixture
def xex():
	if not SAMPLE_XEX.exists():
		pytest.skip("no sample XEX fixture")
	return xex2.Xex2.parse(SAMPLE_XEX.read_bytes())


def test_security_info_getters(xex):
	assert xex.load_address != 0
	assert xex.image_size > 0
	assert xex.header_size > 0
	assert xex.page_descriptor_count > 0


def test_image_info_flag_bits(xex):
	# The ImageInfo bitflags are u32 — just verify they load.
	assert isinstance(xex.image_flags, int)
	assert isinstance(xex.module_flags, int)
	assert isinstance(xex.allowed_media_types, int)
	assert isinstance(xex.game_regions, int)


def test_optional_scalar_absence_raises(xex):
	# afplayer.xex is known to lack these optional headers; make sure
	# absence surfaces as a typed RuntimeError, not a silent 0. The
	# error messages carry the CamelCase header name from `xex2`.
	for attr, header in (
		("default_stack_size", "DefaultStackSize"),
		("default_heap_size", "DefaultHeapSize"),
		("default_fs_cache_size", "DefaultFsCacheSize"),
	):
		with pytest.raises(RuntimeError, match=header):
			getattr(xex, attr)


def test_entry_point_and_original_base(xex):
	# afplayer.xex has both of these.
	assert xex.entry_point != 0
	assert xex.original_base_address != 0


def test_file_format_info(xex):
	# CompressionType: 0=None, 1=Basic, 2=Normal, 3=Delta.
	assert xex.compression_type in (0, 1, 2, 3)
	assert xex.encryption_type in (0, 1)


def test_fixed_size_byte_fields(xex):
	assert len(xex.image_hash) == 20
	assert len(xex.import_table_hash) == 20
	assert len(xex.header_hash) == 20
	assert len(xex.media_id) == 16
	assert len(xex.file_key) == 16
	assert len(xex.rsa_signature) == 256


def test_imports_iteration(xex):
	imps = xex.imports()
	assert len(imps) > 0
	for i in range(len(imps)):
		lib = imps[i]
		assert lib.name  # non-empty string
		assert len(lib.digest) == 20
		# Exercise record access on the first library.
		if i == 0 and len(lib) > 0:
			_ = lib.record_at(0)
			with pytest.raises(RuntimeError, match="out of range"):
				lib.record_at(10**9)


def test_resources_iteration(xex):
	res = xex.resources()
	# Not every XEX ships resources, but afplayer.xex does.
	for i in range(len(res)):
		r = res[i]
		assert r.name
		assert r.size > 0


def test_imports_oob_index_raises(xex):
	imps = xex.imports()
	with pytest.raises(RuntimeError, match="out of range"):
		imps[10**9]


def test_extract_basefile_produces_pe(xex):
	pe = xex.extract_basefile()
	assert pe[:2] == b"MZ"
	assert len(pe) > 0


def test_modify_roundtrips(xex):
	# Apply "remove all" and make sure the output re-parses.
	limits = xex2.Xex2RemoveLimits.all()
	patched = xex.modify(limits)
	assert len(patched) > 0
	re = xex2.Xex2.parse(patched)
	assert re.load_address == xex.load_address


def test_modify_with_empty_limits_is_noop_parseable(xex):
	limits = xex2.Xex2RemoveLimits()
	out = xex.modify(limits)
	# No flags set → no-op patch; output must still be valid.
	xex2.Xex2.parse(out)
