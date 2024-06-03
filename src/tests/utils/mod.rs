pub mod preprocess_asm;
pub mod storage;
pub mod testing_tracer;

/// Enum holding the types of storage refunds
#[derive(Debug, Copy, Clone)]
pub(crate) enum StorageRefund {
    Cold,
    Warm,
}
