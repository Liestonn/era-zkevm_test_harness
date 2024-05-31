use std::{collections::{HashMap, HashSet}, sync::{Arc, Mutex}};

use circuit_definitions::{ethereum_types::{Address, H160}, zk_evm::{
    abstractions::{Storage, StorageAccessRefund},
    aux_structures::{LogQuery, PubdataCost, Timestamp},
    reference_impls::{event_sink::ApplicationData, memory::SimpleMemory},
    testing::{storage::InMemoryStorage, NUM_SHARDS},
    tracing::{AfterDecodingData, AfterExecutionData, BeforeExecutionData, Tracer, VmLocalStateData}, vm_state::PrimitiveValue,
}};
use zkevm_assembly::zkevm_opcode_defs::{
    decoding::{AllowedPcOrImm, EncodingModeProduction, VmEncodingMode}, AddOpcode, DecodedOpcode, NopOpcode, Opcode, PtrOpcode, RetOpcode, MAX_PUBDATA_COST_PER_QUERY, STORAGE_ACCESS_COLD_READ_COST, STORAGE_ACCESS_COLD_WRITE_COST, STORAGE_ACCESS_WARM_READ_COST, STORAGE_ACCESS_WARM_WRITE_COST, STORAGE_AUX_BYTE, TRANSIENT_STORAGE_AUX_BYTE
};

use crate::ethereum_types::U256;

use super::{preprocess_asm::{EXCEPTION_PREFIX, PRINT_PREFIX, PRINT_PTR_PREFIX, PRINT_REG_PREFIX}, testing_tracer::{OutOfCircuitException, TestingTracer}};

#[derive(Debug, Clone)]
pub struct InMemoryCustomRefundStorage
{
    pub inner: [HashMap<Address, HashMap<U256, U256>>; NUM_SHARDS],
    pub inner_transient: [HashMap<Address, HashMap<U256, U256>>; NUM_SHARDS],
    pub cold_warm_markers: [HashMap<Address, HashSet<U256>>; NUM_SHARDS],
    pub transient_cold_warm_markers: [HashMap<Address, HashSet<U256>>; NUM_SHARDS], // not used
    pub frames_stack: Vec<ApplicationData<LogQuery>>,
    pub slot_refund: Option<Arc<Mutex<(u32, u32)>>>,
}

impl InMemoryCustomRefundStorage {
    pub fn new(slot_refund: Option<Arc<Mutex<(u32, u32)>>>) -> Self {
        Self {
            inner: [(); NUM_SHARDS].map(|_| HashMap::default()),
            inner_transient: [(); NUM_SHARDS].map(|_| HashMap::default()),
            cold_warm_markers: [(); NUM_SHARDS].map(|_| HashMap::default()),
            transient_cold_warm_markers: [(); NUM_SHARDS].map(|_| HashMap::default()),
            frames_stack: vec![ApplicationData::empty()],
            slot_refund,
        }
    }

    pub fn populate(&mut self, elements: Vec<(u8, Address, U256, U256)>) {
        for (shard_id, address, key, value) in elements.into_iter() {
            let shard_level_map = &mut self.inner[shard_id as usize];
            let address_level_map = shard_level_map.entry(address).or_default();
            address_level_map.insert(key, value);
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
            match &self.slot_refund {
                None => StorageAccessRefund::Cold,
                Some(val) => {
                    let (is_warm, val) = *val.lock().unwrap();

                    if is_warm == 0 {
                        dbg!(StorageAccessRefund::Cold)
                    } else {
                        dbg!(StorageAccessRefund::Warm { ergs: val })
                    }
                }
            }
    }

    #[track_caller]
    fn execute_partial_query(
        &mut self,
        _monotonic_cycle_counter: u32,
        mut query: LogQuery,
    ) -> (LogQuery, PubdataCost) {
        let aux_byte = query.aux_byte;
        let shard_level_map = if aux_byte == STORAGE_AUX_BYTE {
            &mut self.inner[query.shard_id as usize]
        } else {
            &mut self.inner_transient[query.shard_id as usize]
        };
        let shard_level_warm_map = if aux_byte == STORAGE_AUX_BYTE {
            &mut self.cold_warm_markers[query.shard_id as usize]
        } else {
            &mut self.transient_cold_warm_markers[query.shard_id as usize]
        };
        let frame_data = self.frames_stack.last_mut().expect("frame must be started");

        assert!(!query.rollback);
        if query.rw_flag {
            // write, also append rollback
            let address_level_map = shard_level_map.entry(query.address).or_default();
            let current_value = address_level_map
                .get(&query.key)
                .copied()
                .unwrap_or(U256::zero());
            address_level_map.insert(query.key, query.written_value);

            // mark as warm, and return
            let address_level_warm_map = shard_level_warm_map.entry(query.address).or_default();
            let warm = address_level_warm_map.contains(&query.key);
            if !warm {
                address_level_warm_map.insert(query.key);
            }
            query.read_value = current_value;

            frame_data.forward.push(query);
            query.rollback = true;
            frame_data.rollbacks.push(query);
            query.rollback = false;

            let pubdata_cost = if aux_byte == STORAGE_AUX_BYTE {
                PubdataCost(MAX_PUBDATA_COST_PER_QUERY)
            } else {
                PubdataCost(0i32)
            };

            (query, pubdata_cost)
        } else {
            // read, do not append to rollback
            let address_level_map = shard_level_map.entry(query.address).or_default();
            let current_value = address_level_map
                .get(&query.key)
                .copied()
                .unwrap_or(U256::zero());
            // mark as warm, and return
            let address_level_warm_map = shard_level_warm_map.entry(query.address).or_default();
            let warm = address_level_warm_map.contains(&query.key);
            if !warm {
                address_level_warm_map.insert(query.key);
            }
            query.read_value = current_value;
            frame_data.forward.push(query);

            (query, PubdataCost(0i32))
        }
    }

    #[track_caller]
    fn start_frame(&mut self, _timestamp: Timestamp) {
        let new = ApplicationData::empty();
        self.frames_stack.push(new);
    }

    #[track_caller]
    fn finish_frame(&mut self, _timestamp: Timestamp, panicked: bool) {
        // if we panic then we append forward and rollbacks to the forward of parent,
        // otherwise we place rollbacks of child before rollbacks of the parent
        let current_frame = self
            .frames_stack
            .pop()
            .expect("frame must be started before finishing");
        let ApplicationData { forward, rollbacks } = current_frame;
        let parent_data = self
            .frames_stack
            .last_mut()
            .expect("parent_frame_must_exist");
        if panicked {
            // perform actual rollback
            for query in rollbacks.iter().rev() {
                let LogQuery {
                    shard_id,
                    address,
                    key,
                    read_value,
                    written_value,
                    aux_byte,
                    ..
                } = *query;
                let shard_level_map = if aux_byte == STORAGE_AUX_BYTE {
                    &mut self.inner[shard_id as usize]
                } else {
                    &mut self.inner_transient[shard_id as usize]
                };
                let address_level_map = shard_level_map
                    .get_mut(&address)
                    .expect("must always exist on rollback");
                let current_value_ref = address_level_map
                    .get_mut(&key)
                    .expect("must always exist on rollback");
                assert_eq!(*current_value_ref, written_value); // compare current value
                *current_value_ref = read_value; // write back an old value
            }

            parent_data.forward.extend(forward);
            // add to forward part, but in reverse order
            parent_data.forward.extend(rollbacks.into_iter().rev());
        } else {
            parent_data.forward.extend(forward);
            // we need to prepend rollbacks. No reverse here, as we do not care yet!
            parent_data.rollbacks.extend(rollbacks);
        }
    }

    #[track_caller]
    fn start_new_tx(&mut self, _: Timestamp) {
        for transient in self.inner_transient.iter_mut() {
            transient.clear();
        }
    }
}

