use crate::{
    circuit_input_builder::{
        CircuitInputStateRef, CopyDataType, CopyEvent, ExecStep, NumberOrHash,
    },
    Error,
};
use eth_types::{GethExecStep, Word, U256, StackWord};
use ethers_core::utils::keccak256;
use eth_types::evm_types::MemoryAddress;

use super::Opcode;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sha3;

impl Opcode for Sha3 {
    fn gen_associated_ops(
        state: &mut CircuitInputStateRef,
        geth_steps: &[GethExecStep],
    ) -> Result<Vec<ExecStep>, Error> {
        let geth_step = &geth_steps[0];
        let mut exec_step = state.new_step(geth_step)?;

        // byte offset in the memory.
        let dest = geth_step.stack.nth_last(0)?;
        state.stack_read(&mut exec_step, geth_step.stack.nth_last_filled(0), dest)?;

        // byte size to read in the memory.
        let size = geth_step.stack.nth_last(1)?;
        state.stack_read(&mut exec_step, geth_step.stack.nth_last_filled(1), size)?;

        // byte offset in the memory.
        let offset = geth_step.stack.nth_last(2)?;
        state.stack_read(&mut exec_step, geth_step.stack.nth_last_filled(2), offset)?;

        let expected_sha3 = geth_steps[1].global_memory.read_u256(dest)?;

        if size.gt(&StackWord::zero()) {
            state
                .call_ctx_mut()?
                .memory
                .extend_at_least(offset.as_usize() + size.as_usize());
        }

        let memory = geth_steps[0].global_memory.read_chunk(offset.low_u64().into(), size.as_usize().into());

        // keccak-256 hash of the given data in memory.
        let sha3 = keccak256(&memory);
        debug_assert_eq!(Word::from_big_endian(&sha3), expected_sha3);
        state.memory_write_n(
            &mut exec_step,
            MemoryAddress::from(dest.as_u64()),
            &sha3,
        )?;

        // Memory read operations
        let rw_counter_start = state.block_ctx.rwc;
        let mut steps = Vec::with_capacity(size.as_usize());
        for (i, byte) in memory.iter().enumerate() {
            // Read step
            state.memory_read(&mut exec_step, (offset.as_usize() + i).into(), *byte)?;
            steps.push((*byte, false));
        }
        state.block.sha3_inputs.push(memory);

        let call_id = state.call()?.call_id;
        state.push_copy(
            &mut exec_step,
            CopyEvent {
                src_addr: offset.low_u64(),
                src_addr_end: offset
                    .low_u64()
                    .checked_add(size.as_u64())
                    .unwrap_or(u64::MAX),
                src_type: CopyDataType::Memory,
                src_id: NumberOrHash::Number(call_id),
                dst_addr: 0,
                dst_type: CopyDataType::RlcAcc,
                dst_id: NumberOrHash::Number(call_id),
                log_id: None,
                rw_counter_start,
                bytes: steps,
            },
        );

        Ok(vec![exec_step])
    }
}

#[cfg(any(feature = "test", test))]
pub mod sha3_tests {
    use eth_types::{bytecode, evm_types::OpcodeId, geth_types::GethData, Bytecode, Word, StackWord, bytecode_internal};
    use ethers_core::utils::keccak256;
    use mock::{
        test_ctx::helpers::{account_0_code_account_1_no_code, tx_from_1_to_0},
        TestContext,
    };
    use rand::{random, Rng};

    use crate::{
        circuit_input_builder::{CircuitsParams, ExecState},
        mock::BlockData,
        operation::{MemoryOp, StackOp, RW},
    };

    /// Generate bytecode for SHA3 opcode after having populated sufficient
    /// memory given the offset and size arguments for SHA3.
    pub fn gen_sha3_code(offset: usize, size: usize, mem_kind: MemoryKind) -> (Bytecode, Vec<u8>) {
        let mut rng = rand::thread_rng();
        let data_len = match mem_kind {
            MemoryKind::LessThanSize => {
                offset
                    + if size.gt(&0) {
                    rng.gen_range(0..size)
                } else {
                    0
                }
            }
            MemoryKind::EqualToSize => offset + size,
            MemoryKind::MoreThanSize => {
                offset
                    + size
                    + if size.gt(&0) {
                    rng.gen_range(0..size)
                } else {
                    0
                }
            }
            MemoryKind::Empty => 0,
        };
        let data = rand_bytes(data_len);

        // add opcodes to populate memory in the current context.
        let mut code = Bytecode::default();
        let dest_offset = code.alloc_default_global_data(32);
        code.fill_default_global_data(data.clone());
        // append SHA3 related opcodes at the tail end.
        bytecode_internal! {code,
            I32Const[offset]
            I32Const[size]
            I32Const[dest_offset]
            SHA3
        }
        (code, data)
    }

    /// Memory of a context with respect to the input size to SHA3.
    pub enum MemoryKind {
        /// Variant defining empty memory.
        Empty,
        /// Variant defining memory length being less than size.
        LessThanSize,
        /// Variant defining memory length being equal to size.
        EqualToSize,
        /// Variant defining memory length being more than size.
        MoreThanSize,
    }

    fn rand_bytes(size: usize) -> Vec<u8> {
        (0..size).map(|_| random()).collect::<Vec<u8>>()
    }

    fn test_ok(offset: usize, size: usize, mem_kind: MemoryKind) {
        let (code, memory) = gen_sha3_code(offset, size, mem_kind);
        let memory_len = memory.len();

        // The memory that is hashed.
        let mut memory_view = memory
            .into_iter()
            .skip(offset)
            .take(size)
            .collect::<Vec<u8>>();
        memory_view.resize(size, 0);
        let expected_sha3_value = keccak256(&memory_view);

        let block: GethData = TestContext::<2, 1>::new(
            None,
            account_0_code_account_1_no_code(code),
            tx_from_1_to_0,
            |block, _txs| block,
        )
            .unwrap()
            .into();

        let mut builder = BlockData::new_from_geth_data_with_params(
            block.clone(),
            CircuitsParams {
                max_rws: 2048,
                ..Default::default()
            },
        )
            .new_circuit_input_builder();
        builder
            .handle_block(&block.eth_block, &block.geth_traces)
            .unwrap();

        let step = builder.block.txs()[0]
            .steps()
            .iter()
            .find(|step| step.exec_state == ExecState::Op(OpcodeId::SHA3))
            .unwrap();

        let call_id = builder.block.txs()[0].calls()[0].call_id;

        // Memory reads.
        // Initial memory_len bytes are the memory writes from MSTORE instruction, so we
        // skip them.
        assert_eq!(
            (memory_len..(memory_len + size))
                .map(|idx| &builder.block.container.memory[idx])
                .map(|op| (op.rw(), op.op().clone()))
                .collect::<Vec<(RW, MemoryOp)>>(),
            {
                let mut memory_ops = Vec::with_capacity(size);
                (0..size).for_each(|idx| {
                    let value = memory_view[idx];
                    memory_ops.push((
                        RW::READ,
                        MemoryOp::new(call_id, (offset + idx).into(), value),
                    ));
                });
                memory_ops
            },
        );

        let copy_events = builder.block.copy_events.clone();

        // single copy event with `size` reads and `size` writes.
        assert_eq!(copy_events.len(), 1);
        assert_eq!(copy_events[0].bytes.len(), size);

        for (idx, (value, is_code)) in copy_events[0].bytes.iter().enumerate() {
            assert_eq!(Some(value), memory_view.get(idx));
            assert!(!is_code);
        }
    }

    #[test]
    fn sha3_opcode_ok() {
        test_ok(0x10, 0x32, MemoryKind::Empty);
        test_ok(0x34, 0x44, MemoryKind::LessThanSize);
        test_ok(0x222, 0x111, MemoryKind::EqualToSize);
        test_ok(0x20, 0x30, MemoryKind::MoreThanSize);
    }
}
