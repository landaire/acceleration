//! Changes to a volume: new directories and files, file data, deletion, renames, attributes and
//! timestamps, written as the console's FATX driver writes them.
//!
//! An [`Update`] runs in two phases. It first reads the directories along the paths it names.
//! It then checks the change against the driver's rules and the free space, stages every FAT
//! entry and record it changes, and only then names its writes, in an order chosen so that a
//! write that fails part way, or a power loss, leaves a volume the driver mounts and the reader
//! lists without warnings. An [`Effect::Flush`] barrier separates each stage from the next:
//!
//! - New clusters are marked allocated in the FAT before data or records point to them, and the
//!   new clusters' FAT entries are durable before the entry that links them to an existing
//!   chain is written.
//! - File data and new directory clusters are written before the record that makes them
//!   visible. A record's FileNameLength byte is written last and alone, after the rest of the
//!   record is durable. A live record being rewritten is first hidden by a one-byte write of
//!   the deleted marker, so a write cut at any byte never leaves a live record with a torn
//!   first cluster or size.
//! - A record stops pointing to clusters before the FAT frees them. A truncated chain's new
//!   end is durable before its tail is freed.
//! - A moved entry's old record is marked deleted before its new record is written, as the
//!   driver does; two live records of one entry would share its clusters.
//!
//! An interrupted change can leave allocated clusters that no live record holds, such as those
//! of a record hidden part way through a rewrite or a move, but never a record that points to
//! free clusters.
//! The volume's in-memory FAT changes only when every write succeeded; after a failed write,
//! reopen the volume.
//!
//! Clusters come from those the image holds. A volume file whose FAT covers a longer partition
//! than the file, such as a flash `.fdf` file, gets no clusters past the end of the file.

pub(crate) mod ops;
pub(crate) mod plan;
pub mod request;
pub(crate) mod resolve;

use std::collections::BTreeMap;
use std::collections::VecDeque;

use crate::error::FatxError;
use crate::fat::ClusterNumber;
use crate::fat::FatEntry;
use crate::io;
use crate::io::AsyncReadAt;
use crate::io::AsyncWriteAt;
use crate::io::ReadAt;
use crate::io::WriteAt;
use crate::machine::Effect;
use crate::machine::WriteMachine;
use crate::update::plan::PendingWrite;
use crate::update::request::Change;
use crate::update::request::Request;
use crate::update::resolve::Resolver;
use crate::volume::Volume;

impl Volume {
	/// A machine that makes one change to the volume and its in-memory FAT.
	pub fn updater<'v, 'd>(&'v mut self, request: Request<'d>) -> Update<'v, 'd> {
		let resolver = Resolver::new(request.targets());

		Update { volume: self, request, state: State::Resolving(resolver) }
	}

	/// Makes one change. Nothing is written when the change fails its checks.
	pub fn update<S>(&mut self, source: &S, request: Request<'_>) -> Result<Change, FatxError>
	where
		S: ReadAt + WriteAt,
	{
		io::drive_write(self.updater(request), source)
	}

	pub async fn update_async<S>(&mut self, source: &S, request: Request<'_>) -> Result<Change, FatxError>
	where
		S: AsyncReadAt + AsyncWriteAt,
	{
		io::drive_write_async(self.updater(request), source).await
	}
}

/// Reads the directories a change needs, then writes the change; see the module
/// documentation.
#[derive(Debug)]
pub struct Update<'v, 'd> {
	volume: &'v mut Volume,
	request: Request<'d>,
	state: State<'d>,
}

#[derive(Debug)]
enum State<'d> {
	Resolving(Resolver),
	Writing {
		writes: VecDeque<PendingWrite<'d>>,
		current: Option<PendingWrite<'d>>,
		fat: BTreeMap<ClusterNumber, FatEntry>,
		change: Change,
	},
	Finished,
}

impl<'d> WriteMachine for Update<'_, 'd> {
	type Output = Change;
	type Error = FatxError;

	fn step(&mut self) -> Result<Effect<'_, Change>, FatxError> {
		if let State::Resolving(resolver) = &mut self.state {
			if let Some(range) = resolver.step(self.volume)? {
				return Ok(Effect::Read(range));
			}

			let State::Resolving(resolver) = std::mem::replace(&mut self.state, State::Finished) else {
				unreachable!("matched Resolving");
			};
			let (images, resolved) = resolver.finish();
			let planned = ops::plan(self.volume, &self.request, images, &resolved)?;
			self.state = State::Writing {
				writes: planned.writes.into(),
				current: None,
				fat: planned.fat,
				change: planned.change,
			};
		}

		if matches!(&self.state, State::Writing { writes, .. } if writes.is_empty()) {
			let State::Writing { fat, change, .. } = std::mem::replace(&mut self.state, State::Finished) else {
				unreachable!("matched Writing");
			};
			let table = self.volume.fat_mut();
			for (cluster, entry) in fat {
				table.set(cluster, entry);
			}
			return Ok(Effect::Done(change));
		}

		let State::Writing { writes, current, .. } = &mut self.state else {
			panic!("Update stepped after completion");
		};
		*current = writes.pop_front();

		Ok(current.as_ref().expect("a write was queued").effect())
	}

	fn feed(&mut self, bytes: &[u8]) -> Result<(), FatxError> {
		let State::Resolving(resolver) = &mut self.state else {
			panic!("Update fed without a requested read");
		};

		resolver.feed(bytes)
	}
}
