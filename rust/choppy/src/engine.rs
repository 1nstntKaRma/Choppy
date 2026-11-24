//! The copy engine: Paths, Settings, XxHash64, Model, VolumeProbe, UsbLink, Scanner, Plan (step 1);
//! BigCopy, CopyJob, Handoff, JobManager (step 2).
#![deny(unsafe_code)]

pub mod big_copy;
pub mod copy_job;
pub mod handoff;
pub mod model;
pub mod netfmt;
pub mod paths;
pub mod plan;
pub mod scanner;
pub mod settings;
pub mod usb_link;
pub mod volume_probe;
pub mod xxhash64;
