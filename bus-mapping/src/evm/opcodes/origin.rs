use eth_types::evm_types::MemoryAddress;
use super::Opcode;
use crate::circuit_input_builder::{CircuitInputStateRef, ExecStep};
use crate::operation::CallContextField;
use crate::Error;
use eth_types::GethExecStep;

#[derive(Debug, Copy, Clone)]
pub(crate) struct Origin;

impl Opcode for Origin {
    fn gen_associated_ops(
        state: &mut CircuitInputStateRef,
        geth_steps: &[GethExecStep],
    ) -> Result<Vec<ExecStep>, Error> {
        let step = &geth_steps[0];
        let second_step = &geth_steps[1];
        let mut exec_step = state.new_step(step)?;
        // Get origin result from next step
        let value = &second_step.memory.0;
        let tx_id = state.tx_ctx.id();

        // CallContext read of the TxId
        state.call_context_read(
            &mut exec_step,
            state.call()?.call_id,
            CallContextField::TxId,
            tx_id.into(),
        );

        // Read dest offset as the last stack element
        let dest_offset = step.stack.nth_last(0)?;
        state.stack_read(&mut exec_step, step.stack.nth_last_filled(0), dest_offset)?;
        let offset_addr = MemoryAddress::try_from(dest_offset)?;

        // Copy result to memory
        for i in 0..20 {
            state.memory_write(&mut exec_step, offset_addr.map(|a| a + i), value[i])?;
        }
        let call_ctx = state.call_ctx_mut()?;
        call_ctx.memory = second_step.memory.clone();

        Ok(vec![exec_step])
    }
}

#[cfg(test)]
mod origin_tests {
    use crate::{
        circuit_input_builder::ExecState,
        evm::OpcodeId,
        mock::BlockData,
        operation::{CallContextField, CallContextOp, StackOp, RW},
        Error,
    };
    use eth_types::{bytecode, evm_types::StackAddress, geth_types::GethData, ToWord, Word};
    use mock::{
        test_ctx::{helpers::*, TestContext},
        MOCK_ACCOUNTS,
    };
    use pretty_assertions::assert_eq;

    #[test]
    fn origin_opcode_impl() -> Result<(), Error> {
        let code = bytecode! {
            I32Const(0x78)
            ORIGIN
        };

        // Get the execution steps from the external tracer
        let block: GethData = TestContext::<2, 1>::new(
            None,
            account_0_code_account_1_no_code(code),
            tx_from_1_to_0,
            |block, _tx| block.number(0xcafeu64),
        )
        .unwrap()
        .into();

        let mut builder = BlockData::new_from_geth_data(block.clone()).new_circuit_input_builder();
        builder
            .handle_block(&block.eth_block, &block.geth_traces)
            .unwrap();

        let step = builder.block.txs()[0]
            .steps()
            .iter()
            .find(|step| step.exec_state == ExecState::Op(OpcodeId::ORIGIN))
            .unwrap();

        let op_origin = &builder.block.container.stack[step.bus_mapping_instance[1].as_usize()];
        assert_eq!(step.bus_mapping_instance.len(), 22);
        assert_eq!(
            (op_origin.rw(), op_origin.op()),
            (
                RW::WRITE,
                &StackOp::new(1, StackAddress(1023usize), MOCK_ACCOUNTS[1].to_word())
            )
        );
        let call_id = builder.block.txs()[0].calls()[0].call_id;
        assert_eq!(
            {
                let operation =
                    &builder.block.container.call_context[step.bus_mapping_instance[0].as_usize()];
                (operation.rw(), operation.op())
            },
            (
                RW::READ,
                &CallContextOp {
                    call_id,
                    field: CallContextField::TxId,
                    value: Word::one(),
                }
            )
        );
        Ok(())
    }
}