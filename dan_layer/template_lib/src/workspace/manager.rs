use tari_template_abi::{call_engine, EngineOp};

use crate::{
    args::{InvokeResult, WorkspaceAction, WorkspaceInvokeArg},
    prelude::{Bucket, BucketId},
};

pub struct WorkspaceManager {}

impl WorkspaceManager {
    pub fn list_buckets() -> Vec<BucketId> {
        let resp: InvokeResult = call_engine(EngineOp::WorkspaceInvoke, &WorkspaceInvokeArg {
            action: WorkspaceAction::ListBuckets,
            args: vec![],
        })
        .expect("WorkspaceInvoke returned null");
        let bucket_ids = resp.decode().expect("Failed to decode list of BucketIds");
        bucket_ids
    }

    pub fn take_bucket(bucket_id: BucketId) -> Bucket {
        todo!("This does not work as expected");
        let resp: InvokeResult = call_engine(EngineOp::WorkspaceInvoke, &WorkspaceInvokeArg {
            action: WorkspaceAction::Take,
            args: invoke_args!(bucket_id),
        })
        .expect("Workspace invoke returned null");
        let bucket = resp.decode().expect("Failed to decode Bucket");
        bucket
    }
}
