use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use circuit_definitions::{
    ethereum_types::{Address, H160},
    zk_evm::{
        abstractions::{Storage, StorageAccessRefund},
        aux_structures::{LogQuery, PubdataCost, Timestamp},
        reference_impls::{event_sink::ApplicationData, memory::SimpleMemory},
        testing::{storage::InMemoryStorage, NUM_SHARDS},
        tracing::{
            AfterDecodingData, AfterExecutionData, BeforeExecutionData, Tracer, VmLocalStateData,
        },
        vm_state::PrimitiveValue,
    },
};
use zkevm_assembly::zkevm_opcode_defs::{
    decoding::{AllowedPcOrImm, EncodingModeProduction, VmEncodingMode},
    AddOpcode, DecodedOpcode, NopOpcode, Opcode, PtrOpcode, RetOpcode, MAX_PUBDATA_COST_PER_QUERY,
    STORAGE_ACCESS_COLD_READ_COST, STORAGE_ACCESS_COLD_WRITE_COST, STORAGE_ACCESS_WARM_READ_COST,
    STORAGE_ACCESS_WARM_WRITE_COST, STORAGE_AUX_BYTE, TRANSIENT_STORAGE_AUX_BYTE,
};

use crate::ethereum_types::U256;

/// Enum holding the types of storage refunds
#[derive(Debug, Copy, Clone)]
pub(crate) enum StorageRefund {
    Cold,
    Warm,
}

#[derive(Debug, Clone)]
pub struct InMemoryCustomRefundStorage {
    pub storage: InMemoryStorage,
    pub slot_refund: Arc<Mutex<(StorageRefund, u32)>>,
}

impl InMemoryCustomRefundStorage {
    pub fn new() -> Self {
        Self {
            storage: InMemoryStorage::new(),
            slot_refund: Arc::new(Mutex::new((StorageRefund::Cold, 0u32))),
        }
    }
}

impl Storage for InMemoryCustomRefundStorage {
    #[track_caller]
    fn get_access_refund(
        &mut self, // to avoid any hacks inside, like prefetch
        _monotonic_cycle_counter: u32,
        _partial_query: &LogQuery,
    ) -> StorageAccessRefund {
        let storage_refund = &self.slot_refund.lock().unwrap();
        match storage_refund.0 {
            StorageRefund::Cold => dbg!(StorageAccessRefund::Cold),
            StorageRefund::Warm => dbg!(StorageAccessRefund::Warm {
                ergs: storage_refund.1
            }),
        }
    }

    #[track_caller]
    fn execute_partial_query(
        &mut self,
        monotonic_cycle_counter: u32,
        query: LogQuery,
    ) -> (LogQuery, PubdataCost) {
        self.storage
            .execute_partial_query(monotonic_cycle_counter, query)
    }

    #[track_caller]
    fn start_frame(&mut self, timestamp: Timestamp) {
        self.storage.start_frame(timestamp)
    }

    #[track_caller]
    fn finish_frame(&mut self, timestamp: Timestamp, panicked: bool) {
        self.storage.finish_frame(timestamp, panicked)
    }

    #[track_caller]
    fn start_new_tx(&mut self, timestamp: Timestamp) {
        self.storage.start_new_tx(timestamp)
    }
}
