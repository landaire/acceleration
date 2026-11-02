/// The `N` bytes of `bytes` at `offset`. The caller has checked the length.
pub(crate) fn array<const N: usize>(bytes: &[u8], offset: usize) -> [u8; N] {
	let mut field = [0; N];
	field.copy_from_slice(&bytes[offset..offset + N]);

	field
}
