// This directory's extension (hand-written; ADR-0031). The generated mod.rs (lib.rs for
// src/) pulls it in with include!, so its lines resolve against this directory.

// The hand-written payment SQL's parameter/projection types (see `payment_entry_repository` and
// `payment_allocation_repository`, both declared `user_owned` in metaphor.codegen.yaml). The param
// structs mirror COLUMNS, not entities.
pub use payment_allocation_repository::{AllocationRow, NewAllocationRow};
pub use payment_entry_repository::{
    NewPaymentEntryRow, PaymentTypeAmountRow, PostSourceRow, PostedStateRow, SettledHeaderRow,
};
// The hand-written dunning SQL's parameter/projection types (see `aging_snapshot_repository`,
// `aging_bucket_repository`, `dunning_run_repository`, `dunning_action_repository`, all declared
// `user_owned` in metaphor.codegen.yaml). The param structs mirror COLUMNS, not entities.
pub use aging_bucket_repository::NewAgingBucketRow;
pub use aging_snapshot_repository::AgingTotals;
pub use dunning_action_repository::NewDunningActionRow;
