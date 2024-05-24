use circuit_definitions::zk_evm::vm_state::ErrorFlags;
use circuit_definitions::zk_evm::vm_state::PrimitiveValue;
use zkevm_assembly::zkevm_opcode_defs::AddOpcode;
use zkevm_assembly::zkevm_opcode_defs::NopOpcode;
use zkevm_assembly::zkevm_opcode_defs::Opcode;
use zkevm_assembly::zkevm_opcode_defs::PtrOpcode;
use zkevm_assembly::zkevm_opcode_defs::RetOpcode;

use crate::ethereum_types::U256;
use crate::zk_evm::opcodes::DecodedOpcode;
use crate::zk_evm::reference_impls::memory::SimpleMemory;
use crate::zk_evm::tracing::*;

use crate::tests::utils::preprocess_asm::EXCEPTION_PREFIX;
use crate::tests::utils::preprocess_asm::PRINT_PREFIX;
use crate::tests::utils::preprocess_asm::PRINT_PTR_PREFIX;
use crate::tests::utils::preprocess_asm::PRINT_REG_PREFIX;

#[derive(Debug, Clone, PartialEq, Default)]
enum TracerState {
    /// will try to parse next value from VM as command
    #[default]
    ExpectingCommand,
    /// will print next value from VM
    ExpectingValueToPrint(ExpectedValueType),
}

#[derive(Debug, Clone, PartialEq)]
enum ExpectedValueType {
    /// expecting raw register value
    Register,
    /// expecting fat pointer
    Pointer,
}

/// Tracks prints and exceptions during VM execution cycles.
#[derive(Debug, Clone, Default)]
pub struct TestingTracer {
    /// the last uncatched exception message
    pub exception_message: Option<String>,
    /// the inner state, affects the interpretation of values from the VM
    tracer_state: TracerState,
    /// stores pending messages that should be printed
    message_buffer: Option<String>,
}

/// TestingTracer interprets valid x values in `add x r0 r0` and `ptr.add x r0 r0` instructions as commands to execute.
/// Commands have following structure: "PREFIX:arg"
/// Allowed commands:
/// "EXCEPTION_PREFIX:<text>" - save <text> in exception_message field
/// "PRINT_PREFIX:<text>" - print <text> in the console
/// "PRINT_REG_PREFIX:" - print raw "x" value of next command in the console
/// "PRINT_PTR_PREFIX:" - print raw "x" pointer value of next command in the console (currently same result as previous command)
impl TestingTracer {
    fn reset_exception(&mut self) {
        self.exception_message = None;
    }

    fn set_exception_message(&mut self, message: &str) {
        self.exception_message = Some(message.to_owned());
    }

    fn execute_print(&self, message: &str) {
        println!("{}", message);
    }

    fn execute_print_from_register(&self, val: PrimitiveValue) {
        if let TracerState::ExpectingCommand = self.tracer_state {
            panic!("Unexpected execute_print_from_register command");
        }

        if let Some(message) = &self.message_buffer {
            println!("{message} {}", val.value);
        } else {
            println!("{}", val.value);
        }
    }

    fn handle_value_from_vm(&mut self, value: PrimitiveValue) -> TracerState {
        let mut new_state = TracerState::ExpectingCommand;
        let mut new_message_buffer_value = None;

        match self.tracer_state {
            TracerState::ExpectingValueToPrint(..) => {
                self.execute_print_from_register(value);
            }
            TracerState::ExpectingCommand => {
                if let Some((command_prefix, arg)) = self.parse_command_from_register(value) {
                    match command_prefix.as_str() {
                        EXCEPTION_PREFIX => {
                            self.set_exception_message(&arg);
                        }
                        PRINT_PREFIX => {
                            self.execute_print(&arg);
                        }
                        PRINT_REG_PREFIX => {
                            if !arg.is_empty() {
                                new_message_buffer_value = Some(arg);
                            }
                            new_state =
                                TracerState::ExpectingValueToPrint(ExpectedValueType::Register);
                        }
                        PRINT_PTR_PREFIX => {
                            if !arg.is_empty() {
                                new_message_buffer_value = Some(arg);
                            }
                            new_state =
                                TracerState::ExpectingValueToPrint(ExpectedValueType::Pointer);
                        }
                        _ => {
                            // ignore invalid command
                        }
                    }
                }
            }
        }

        self.message_buffer = new_message_buffer_value;
        new_state
    }

    /// Returns (command_prefix, arg) if parsed command successfully
    /// None otherwise
    fn parse_command_from_register(&self, val: PrimitiveValue) -> Option<(String, String)> {
        if val.value == U256::from(0) {
            return None;
        }

        let mut bytes: [u8; 32] = [0; 32];
        val.value.to_big_endian(&mut bytes);

        if let Ok(message) = std::str::from_utf8(&bytes) {
            let message_trimmed = message.trim_matches(char::from(0));

            for prefix in [
                EXCEPTION_PREFIX,
                PRINT_PREFIX,
                PRINT_REG_PREFIX,
                PRINT_PTR_PREFIX,
            ] {
                if message_trimmed.starts_with(prefix) {
                    let arg = message_trimmed.strip_prefix(prefix).unwrap();
                    return Some((prefix.to_owned(), arg.to_owned()));
                }
            }
        }

        None
    }
}

impl Tracer for TestingTracer {
    type SupportedMemory = SimpleMemory;
    const CALL_BEFORE_EXECUTION: bool = true;
    const CALL_AFTER_DECODING: bool = true;

    #[inline]
    fn before_decoding(&mut self, _state: VmLocalStateData<'_>, _memory: &Self::SupportedMemory) {}

    fn after_decoding(
        &mut self,
        _state: VmLocalStateData<'_>,
        data: AfterDecodingData,
        _memory: &Self::SupportedMemory,
    ) {
        // check for built-in panics
        if !data.error_flags_accumulated.is_empty() {
            // last accumulated panic will be used as exception_message
            for (panic, _) in data.error_flags_accumulated.iter_names() {
                self.exception_message = Some(panic.to_owned());
            }
        }
    }

    fn before_execution(
        &mut self,
        _state: VmLocalStateData<'_>,
        data: BeforeExecutionData,
        _memory: &Self::SupportedMemory,
    ) {
        let inner_opcode = data.opcode.inner.variant.opcode;

        // Propagate error message if Nop, ret.panic, ret.revert; reset otherwise
        match inner_opcode {
            Opcode::Nop(NopOpcode) => {}
            Opcode::Ret(RetOpcode::Panic) => {}
            Opcode::Ret(RetOpcode::Revert) => {}
            _ => {
                self.reset_exception();
            }
        }

        // check if we have a valid command for TestingTracer and execute the command if any.
        // commands always have r0 as src1 and dst0
        let new_state = if data.opcode.src1_reg_idx == 0 && data.opcode.dst0_reg_idx == 0 {
            match inner_opcode {
                Opcode::Add(AddOpcode::Add) | Opcode::Ptr(PtrOpcode::Add) => {
                    // `add x r0 r0` is used to pass "x" to TestingTracer
                    // `ptr.add x r0 r0` is used to pass "x" pointer to TestingTracer
                    self.handle_value_from_vm(data.src0_value)
                }
                _ => TracerState::ExpectingCommand,
            }
        } else {
            TracerState::ExpectingCommand
        };

        self.tracer_state = new_state;

        // pc 0 means VM finished without any panics
        if data.new_pc == 0 {
            self.reset_exception();
        }
    }

    #[inline]
    fn after_execution(
        &mut self,
        _state: VmLocalStateData<'_>,
        _data: AfterExecutionData,
        _memory: &Self::SupportedMemory,
    ) {
    }
}
