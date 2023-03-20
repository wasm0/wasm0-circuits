use std::fmt;

use bus_mapping::{
    circuit_input_builder,
    error::{ExecError, OogError},
    evm::OpcodeId,
    operation,
};
use eth_types::evm_unimplemented;

use crate::{
    evm_circuit::{
        param::STACK_CAPACITY,
        step::ExecutionState,
    },
    table::RwTableTag,
};
use crate::evm_circuit::param::N_BYTES_U64;

/// Step executed in a transaction
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ExecStep {
    /// The index in the Transaction calls
    pub call_index: usize,
    /// The indices in the RW trace incurred in this step
    pub rw_indices: Vec<(RwTableTag, usize)>,
    /// Number of rw operations performed via a copy event in this step.
    pub copy_rw_counter_delta: u64,
    /// The execution state for the step
    pub execution_state: ExecutionState,
    /// The Read/Write counter before the step
    pub rw_counter: usize,
    /// The program counter
    pub program_counter: u64,
    /// The stack pointer
    pub stack_pointer: usize,
    /// The amount of gas left
    pub gas_left: u64,
    /// The gas cost in this step
    pub gas_cost: u64,
    /// The memory size in bytes
    pub memory_size: u64,
    /// The counter for reversible writes at the beginning of the step
    pub reversible_write_counter: usize,
    /// The number of reversible writes from this step
    pub reversible_write_counter_delta: usize,
    /// The counter for log index within tx
    pub log_id: usize,
    /// The opcode corresponds to the step
    pub opcode: Option<OpcodeId>,
}

impl fmt::Debug for ExecStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecStep")
            .field("call_index", &self.call_index)
            // .field("rw_indices", &self.rw_indices)
            .field("execution_state", &self.execution_state)
            .field("rw_counter", &self.rw_counter)
            .field("program_counter", &self.program_counter)
            .field("stack_pointer", &self.stack_pointer)
            .field("gas_left", &self.gas_left)
            .field("gas_cost", &self.gas_cost)
            .field("memory_size", &self.memory_size)
            .field("reversible_write_counter", &self.reversible_write_counter)
            .field("log_id", &self.log_id)
            .field("opcode", &self.opcode)
            .finish()
    }
}

impl ExecStep {
    /// The memory size in word **before** this step
    pub fn memory_word_size(&self) -> u64 {
        // EVM always pads the memory size to word size
        // https://github.com/ethereum/go-ethereum/blob/master/core/vm/interpreter.go#L212-L216
        // Thus, the memory size must be a multiple of 32 bytes.
        // TODO wasm0: EVM pads the memory, but it doesnt work with wasm
        // assert_eq!(self.memory_size % N_BYTES_WORD as u64, 0);
        // self.memory_size / N_BYTES_WORD as u64
        // TODO wasm0: what about word size and number below if `self.memory_size / N_BYTES_WORD * N_BYTES_WORD < self.memory_size`
        // temporal fix
        let mut word_count = self.memory_size / N_BYTES_U64 as u64;
        if word_count * (N_BYTES_U64 as u64) < self.memory_size { word_count += 1 }
        word_count
    }
}

impl From<&ExecError> for ExecutionState {
    fn from(error: &ExecError) -> Self {
        match error {
            ExecError::InvalidOpcode => ExecutionState::ErrorInvalidOpcode,
            ExecError::StackOverflow | ExecError::StackUnderflow => ExecutionState::ErrorStack,
            ExecError::WriteProtection => ExecutionState::ErrorWriteProtection,
            ExecError::Depth => ExecutionState::ErrorDepth,
            ExecError::InsufficientBalance => ExecutionState::ErrorInsufficientBalance,
            ExecError::ContractAddressCollision => ExecutionState::ErrorContractAddressCollision,
            ExecError::InvalidCreationCode => ExecutionState::ErrorInvalidCreationCode,
            ExecError::InvalidJump => ExecutionState::ErrorInvalidJump,
            ExecError::ReturnDataOutOfBounds => ExecutionState::ErrorReturnDataOutOfBound,
            ExecError::CodeStoreOutOfGas => ExecutionState::ErrorOutOfGasCodeStore,
            ExecError::MaxCodeSizeExceeded => ExecutionState::ErrorMaxCodeSizeExceeded,
            ExecError::OutOfGas(oog_error) => match oog_error {
                OogError::Constant => ExecutionState::ErrorOutOfGasConstant,
                OogError::StaticMemoryExpansion => {
                    ExecutionState::ErrorOutOfGasStaticMemoryExpansion
                }
                OogError::DynamicMemoryExpansion => {
                    ExecutionState::ErrorOutOfGasDynamicMemoryExpansion
                }
                OogError::MemoryCopy => ExecutionState::ErrorOutOfGasMemoryCopy,
                OogError::AccountAccess => ExecutionState::ErrorOutOfGasAccountAccess,
                OogError::CodeStore => ExecutionState::ErrorOutOfGasCodeStore,
                OogError::Log => ExecutionState::ErrorOutOfGasLOG,
                OogError::Exp => ExecutionState::ErrorOutOfGasEXP,
                OogError::Sha3 => ExecutionState::ErrorOutOfGasSHA3,
                OogError::ExtCodeCopy => ExecutionState::ErrorOutOfGasEXTCODECOPY,
                OogError::Call => ExecutionState::ErrorOutOfGasCall,
                OogError::SloadSstore => ExecutionState::ErrorOutOfGasSloadSstore,
                OogError::Create2 => ExecutionState::ErrorOutOfGasCREATE2,
                OogError::SelfDestruct => ExecutionState::ErrorOutOfGasSELFDESTRUCT,
            },
        }
    }
}

impl From<&circuit_input_builder::ExecStep> for ExecutionState {
    fn from(step: &circuit_input_builder::ExecStep) -> Self {
        if let Some(error) = step.error.as_ref() {
            return error.into();
        }
        match step.exec_state {
            circuit_input_builder::ExecState::Op(op) => {
                if op.is_log() {
                    return ExecutionState::LOG;
                }

                macro_rules! dummy {
                    ($name:expr) => {{
                        evm_unimplemented!("{:?} is implemented with DummyGadget", $name);
                        $name
                    }};
                }

                match op {
                    // WASM opcodes
                    OpcodeId::I32Add |
                    OpcodeId::I64Add |
                    OpcodeId::I32Sub |
                    OpcodeId::I64Sub |
                    OpcodeId::I32Mul |
                    OpcodeId::I64Mul |
                    OpcodeId::I32DivS |
                    OpcodeId::I64DivS |
                    OpcodeId::I32DivU |
                    OpcodeId::I64DivU |
                    OpcodeId::I32RemS |
                    OpcodeId::I64RemS |
                    OpcodeId::I32RemU |
                    OpcodeId::I64RemU => ExecutionState::WASM_BIN,

                    OpcodeId::I32Const |
                    OpcodeId::I64Const => ExecutionState::WASM_CONST,

                    OpcodeId::Drop => ExecutionState::WASM_DROP,

                    OpcodeId::I32Ctz |
                    OpcodeId::I64Ctz |
                    OpcodeId::I32Clz |
                    OpcodeId::I64Clz |
                    OpcodeId::I32Popcnt |
                    OpcodeId::I64Popcnt => ExecutionState::WASM_UNARY,

                    OpcodeId::GetGlobal |
                    OpcodeId::SetGlobal => ExecutionState::WASM_GLOBAL,

                    OpcodeId::GetLocal |
                    OpcodeId::SetLocal |
                    OpcodeId::TeeLocal => ExecutionState::WASM_LOCAL,

                    OpcodeId::End => ExecutionState::WASM_END,

                    // EVM opcodes
                    OpcodeId::ADDMOD => ExecutionState::ADDMOD,
                    OpcodeId::ADDRESS => ExecutionState::ADDRESS,
                    OpcodeId::BALANCE => ExecutionState::BALANCE,
                    OpcodeId::MUL | OpcodeId::DIV | OpcodeId::MOD => ExecutionState::MUL_DIV_MOD,
                    OpcodeId::MULMOD => ExecutionState::MULMOD,
                    OpcodeId::SDIV | OpcodeId::SMOD => ExecutionState::SDIV_SMOD,
                    OpcodeId::EQ | OpcodeId::LT | OpcodeId::GT => ExecutionState::CMP,
                    OpcodeId::SLT | OpcodeId::SGT => ExecutionState::SCMP,
                    OpcodeId::SIGNEXTEND => ExecutionState::SIGNEXTEND,
                    OpcodeId::STOP => ExecutionState::STOP,
                    OpcodeId::AND => ExecutionState::BITWISE,
                    OpcodeId::XOR => ExecutionState::BITWISE,
                    OpcodeId::OR => ExecutionState::BITWISE,
                    OpcodeId::NOT => ExecutionState::NOT,
                    OpcodeId::EXP => ExecutionState::EXP,
                    OpcodeId::POP => ExecutionState::POP,
                    OpcodeId::BYTE => ExecutionState::BYTE,
                    OpcodeId::MLOAD => ExecutionState::MEMORY,
                    OpcodeId::MSTORE => ExecutionState::MEMORY,
                    OpcodeId::MSTORE8 => ExecutionState::MEMORY,
                    OpcodeId::JUMPDEST => ExecutionState::JUMPDEST,
                    OpcodeId::JUMP => ExecutionState::JUMP,
                    OpcodeId::JUMPI => ExecutionState::JUMPI,
                    OpcodeId::GASPRICE => ExecutionState::GASPRICE,
                    OpcodeId::PC => ExecutionState::PC,
                    OpcodeId::MSIZE => ExecutionState::MSIZE,
                    OpcodeId::CALLER => ExecutionState::CALLER,
                    OpcodeId::CALLVALUE => ExecutionState::CALLVALUE,
                    OpcodeId::EXTCODEHASH => ExecutionState::EXTCODEHASH,
                    OpcodeId::EXTCODESIZE => ExecutionState::EXTCODESIZE,
                    OpcodeId::BLOCKHASH => ExecutionState::BLOCKHASH,
                    OpcodeId::TIMESTAMP | OpcodeId::NUMBER | OpcodeId::GASLIMIT => {
                        ExecutionState::BLOCKCTXU64
                    }
                    OpcodeId::COINBASE => ExecutionState::BLOCKCTXU160,
                    OpcodeId::DIFFICULTY | OpcodeId::BASEFEE => ExecutionState::BLOCKCTXU256,
                    OpcodeId::GAS => ExecutionState::GAS,
                    OpcodeId::SAR => ExecutionState::SAR,
                    OpcodeId::SELFBALANCE => ExecutionState::SELFBALANCE,
                    OpcodeId::SHA3 => ExecutionState::SHA3,
                    OpcodeId::SHL | OpcodeId::SHR => ExecutionState::SHL_SHR,
                    OpcodeId::SLOAD => ExecutionState::SLOAD,
                    OpcodeId::SSTORE => ExecutionState::SSTORE,
                    OpcodeId::CALLDATASIZE => ExecutionState::CALLDATASIZE,
                    OpcodeId::CALLDATACOPY => ExecutionState::CALLDATACOPY,
                    OpcodeId::CHAINID => ExecutionState::CHAINID,
                    OpcodeId::ISZERO => ExecutionState::ISZERO,
                    OpcodeId::CALL
                    | OpcodeId::CALLCODE
                    | OpcodeId::DELEGATECALL
                    | OpcodeId::STATICCALL => ExecutionState::CALL_OP,
                    OpcodeId::ORIGIN => ExecutionState::ORIGIN,
                    OpcodeId::CODECOPY => ExecutionState::CODECOPY,
                    OpcodeId::CALLDATALOAD => ExecutionState::CALLDATALOAD,
                    OpcodeId::CODESIZE => ExecutionState::CODESIZE,
                    OpcodeId::RETURN | OpcodeId::REVERT => ExecutionState::RETURN_REVERT,
                    OpcodeId::RETURNDATASIZE => ExecutionState::RETURNDATASIZE,
                    OpcodeId::RETURNDATACOPY => ExecutionState::RETURNDATACOPY,
                    // dummy ops
                    OpcodeId::EXTCODECOPY => dummy!(ExecutionState::EXTCODECOPY),
                    OpcodeId::CREATE => dummy!(ExecutionState::CREATE),
                    OpcodeId::CREATE2 => dummy!(ExecutionState::CREATE2),
                    OpcodeId::SELFDESTRUCT => dummy!(ExecutionState::SELFDESTRUCT),
                    _ => unimplemented!("unimplemented opcode {:?}", op),
                }
            }
            circuit_input_builder::ExecState::BeginTx => ExecutionState::BeginTx,
            circuit_input_builder::ExecState::EndTx => ExecutionState::EndTx,
            circuit_input_builder::ExecState::EndBlock => ExecutionState::EndBlock,
        }
    }
}

pub(super) fn step_convert(step: &circuit_input_builder::ExecStep) -> ExecStep {
    ExecStep {
        call_index: step.call_index,
        rw_indices: step
            .bus_mapping_instance
            .iter()
            .map(|x| {
                let tag = match x.target() {
                    operation::Target::Memory => RwTableTag::Memory,
                    operation::Target::Stack => RwTableTag::Stack,
                    operation::Target::Global => RwTableTag::Global,
                    operation::Target::Storage => RwTableTag::AccountStorage,
                    operation::Target::TxAccessListAccount => RwTableTag::TxAccessListAccount,
                    operation::Target::TxAccessListAccountStorage => {
                        RwTableTag::TxAccessListAccountStorage
                    }
                    operation::Target::TxRefund => RwTableTag::TxRefund,
                    operation::Target::Account => RwTableTag::Account,
                    operation::Target::CallContext => RwTableTag::CallContext,
                    operation::Target::TxReceipt => RwTableTag::TxReceipt,
                    operation::Target::TxLog => RwTableTag::TxLog,
                    operation::Target::Start => RwTableTag::Start,
                };
                (tag, x.as_usize())
            })
            .collect(),
        copy_rw_counter_delta: step.copy_rw_counter_delta,
        execution_state: ExecutionState::from(step),
        rw_counter: usize::from(step.rwc),
        program_counter: usize::from(step.pc) as u64,
        stack_pointer: STACK_CAPACITY - step.stack_size,
        gas_left: step.gas_left.0,
        gas_cost: step.gas_cost.as_u64(),
        opcode: match step.exec_state {
            circuit_input_builder::ExecState::Op(op) => Some(op),
            _ => None,
        },
        memory_size: step.memory_size as u64,
        reversible_write_counter: step.reversible_write_counter,
        reversible_write_counter_delta: step.reversible_write_counter_delta,
        log_id: step.log_id,
    }
}
