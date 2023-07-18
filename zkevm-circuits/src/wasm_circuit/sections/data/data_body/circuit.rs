use std::marker::PhantomData;
use std::rc::Rc;

use halo2_proofs::{
    plonk::{Column, ConstraintSystem},
};
use halo2_proofs::circuit::{Region, Value};
use halo2_proofs::plonk::{Advice, Fixed};
use halo2_proofs::poly::Rotation;
use itertools::Itertools;
use log::debug;

use eth_types::Field;
use gadgets::binary_number::BinaryNumberChip;
use gadgets::util::{and, Expr, not, or};

use crate::evm_circuit::util::constraint_builder::{BaseConstraintBuilder, ConstrainBuilderCommon};
use crate::wasm_circuit::bytecode::bytecode::WasmBytecode;
use crate::wasm_circuit::bytecode::bytecode_table::WasmBytecodeTable;
use crate::wasm_circuit::consts::{MemSegmentType, NumericInstruction, WASM_BLOCK_END};
use crate::wasm_circuit::error::Error;
use crate::wasm_circuit::leb128_circuit::circuit::LEB128Chip;
use crate::wasm_circuit::leb128_circuit::helpers::{leb128_compute_sn, leb128_compute_sn_recovered_at_position};
use crate::wasm_circuit::sections::consts::LebParams;
use crate::wasm_circuit::sections::data::data_body::types::AssignType;
use crate::wasm_circuit::sections::helpers::{configure_transition_check, configure_constraints_for_q_first_and_q_last};
use crate::wasm_circuit::tables::dynamic_indexes::circuit::DynamicIndexesChip;
use crate::wasm_circuit::tables::dynamic_indexes::types::{LookupArgsParams, Tag};
use crate::wasm_circuit::types::SharedState;

#[derive(Debug, Clone)]
pub struct WasmDataSectionBodyConfig<F: Field> {
    pub q_enable: Column<Fixed>,
    pub q_first: Column<Fixed>,
    pub q_last: Column<Fixed>,
    pub is_items_count: Column<Fixed>,
    pub is_memidx: Column<Fixed>,
    pub is_mem_segment_type: Column<Fixed>,
    pub is_mem_segment_size_opcode: Column<Fixed>,
    pub is_mem_segment_size: Column<Fixed>,
    pub is_block_end: Column<Fixed>,
    pub is_mem_segment_len: Column<Fixed>,
    pub is_mem_segment_bytes: Column<Fixed>,

    pub is_mem_segment_type_ctx: Column<Fixed>,

    pub leb128_chip: Rc<LEB128Chip<F>>,
    pub dynamic_indexes_chip: Rc<DynamicIndexesChip<F>>,
    pub mem_segment_type: Column<Advice>,
    pub mem_segment_type_chip: Rc<BinaryNumberChip<F, MemSegmentType, 8>>,

    _marker: PhantomData<F>,
}

impl<'a, F: Field> WasmDataSectionBodyConfig<F>
{}

#[derive(Debug, Clone)]
pub struct WasmDataSectionBodyChip<F: Field> {
    pub config: WasmDataSectionBodyConfig<F>,
    _marker: PhantomData<F>,
}

impl<F: Field> WasmDataSectionBodyChip<F>
{
    pub fn construct(config: WasmDataSectionBodyConfig<F>) -> Self {
        let instance = Self {
            config,
            _marker: PhantomData,
        };
        instance
    }

    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        bytecode_table: Rc<WasmBytecodeTable>,
        leb128_chip: Rc<LEB128Chip<F>>,
        dynamic_indexes_chip: Rc<DynamicIndexesChip<F>>,
    ) -> WasmDataSectionBodyConfig<F> {
        let q_enable = cs.fixed_column();
        let q_first = cs.fixed_column();
        let q_last = cs.fixed_column();
        let is_items_count = cs.fixed_column();
        let is_memidx = cs.fixed_column();
        let is_mem_segment_type = cs.fixed_column();
        let is_mem_segment_size_opcode = cs.fixed_column();
        let is_mem_segment_size = cs.fixed_column();
        let is_block_end = cs.fixed_column();
        let is_mem_segment_len = cs.fixed_column();
        let is_mem_segment_bytes = cs.fixed_column();

        let is_mem_segment_type_ctx = cs.fixed_column();
        let mem_segment_type = cs.advice_column();

        let config = BinaryNumberChip::configure(
            cs,
            is_mem_segment_type_ctx,
            Some(mem_segment_type.into()),
        );
        let mem_segment_type_chip = Rc::new(BinaryNumberChip::construct(config));

        dynamic_indexes_chip.lookup_args(
            "data section has valid setup for data indexes",
            cs,
            |vc| {
                LookupArgsParams {
                    cond: vc.query_fixed(is_items_count, Rotation::cur()),
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::cur()),
                    tag: Tag::DataSectionDataIndex.expr(),
                    is_terminator: true.expr(),
                }
            }
        );

        cs.create_gate("WasmDataSectionBody gate", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_enable_expr = vc.query_fixed(q_enable, Rotation::cur());
            let q_first_expr = vc.query_fixed(q_first, Rotation::cur());
            let q_last_expr = vc.query_fixed(q_last, Rotation::cur());
            let not_q_last_expr = not::expr(q_last_expr.clone());
            let is_items_count_expr = vc.query_fixed(is_items_count, Rotation::cur());
            let is_mem_segment_type_expr = vc.query_fixed(is_mem_segment_type, Rotation::cur());
            let is_mem_index_expr = vc.query_fixed(is_memidx, Rotation::cur());
            let is_mem_segment_size_opcode_expr = vc.query_fixed(is_mem_segment_size_opcode, Rotation::cur());
            let is_mem_segment_size_expr = vc.query_fixed(is_mem_segment_size, Rotation::cur());
            let is_block_end_expr = vc.query_fixed(is_block_end, Rotation::cur());
            let is_mem_segment_len_expr = vc.query_fixed(is_mem_segment_len, Rotation::cur());
            let is_mem_segment_bytes_expr = vc.query_fixed(is_mem_segment_bytes, Rotation::cur());

            let is_mem_segment_type_ctx_prev_expr = vc.query_fixed(is_mem_segment_type_ctx, Rotation::prev());
            let is_mem_segment_type_ctx_expr = vc.query_fixed(is_mem_segment_type_ctx, Rotation::cur());

            let byte_val_expr = vc.query_advice(bytecode_table.value, Rotation::cur());
            let mem_segment_type_expr = vc.query_advice(mem_segment_type, Rotation::cur());

            cb.require_boolean("q_enable is boolean", q_enable_expr.clone());
            cb.require_boolean("is_items_count is boolean", is_items_count_expr.clone());
            cb.require_boolean("is_mem_segment_type is boolean", is_mem_segment_type_expr.clone());
            cb.require_boolean("is_mem_index is boolean", is_mem_index_expr.clone());
            cb.require_boolean("is_mem_segment_size_opcode is boolean", is_mem_segment_size_opcode_expr.clone());
            cb.require_boolean("is_mem_segment_size is boolean", is_mem_segment_size_expr.clone());
            cb.require_boolean("is_block_end is boolean", is_block_end_expr.clone());
            cb.require_boolean("is_mem_segment_len is boolean", is_mem_segment_len_expr.clone());
            cb.require_boolean("is_mem_segment_bytes is boolean", is_mem_segment_bytes_expr.clone());

            cb.require_equal(
                "exactly one mark flag active at the same time",
                is_items_count_expr.clone()
                    + is_mem_segment_type_expr.clone()
                    + is_mem_index_expr.clone()
                    + is_mem_segment_size_opcode_expr.clone()
                    + is_mem_segment_size_expr.clone()
                    + is_block_end_expr.clone()
                    + is_mem_segment_len_expr.clone()
                    + is_mem_segment_bytes_expr.clone()
                ,
                1.expr(),
            );

            cb.condition(
                or::expr([
                    is_items_count_expr.clone(),
                    is_mem_index_expr.clone(),
                    is_mem_segment_size_expr.clone(),
                    is_mem_segment_len_expr.clone(),
                ]),
                |bcb| {
                    bcb.require_equal(
                        "is_items_count || is_mem_index || is_mem_segment_size || is_mem_segment_len -> leb128",
                        vc.query_fixed(leb128_chip.config.q_enable, Rotation::cur()),
                        1.expr(),
                    )
                }
            );

            configure_constraints_for_q_first_and_q_last(
                &mut cb,
                vc,
                &q_enable,
                &q_first,
                &[is_items_count],
                &q_last,
                &[is_mem_segment_len, is_mem_segment_bytes],
            );

            // constraints for IsMemSegmentTypeCtx
            cb.condition(
                is_mem_segment_type_ctx_expr.clone(),
                |bcb| {
                    bcb.require_equal(
                        "is_mem_segment_type_ctx => specific flags are active",
                        is_mem_segment_type_expr.clone()
                            + is_mem_index_expr.clone()
                            + is_mem_segment_size_opcode_expr.clone()
                            + is_mem_segment_size_expr.clone()
                            + is_block_end_expr.clone()
                            + is_mem_segment_len_expr.clone()
                            + is_mem_segment_bytes_expr.clone(),
                        1.expr(),
                    )
                }
            );
            // constraints for AssignType::MemSegmentType
            cb.condition(
                is_mem_segment_type_expr.clone(),
                |bcb| {
                    bcb.require_equal(
                        "is_mem_segment_type => mem_segment_type=byte_val",
                        mem_segment_type_expr.clone(),
                        byte_val_expr.clone(),
                    );
                }
            );
            cb.condition(
                and::expr([
                    is_mem_segment_type_ctx_expr.clone(),
                    is_mem_segment_type_ctx_prev_expr.clone(),
                ]),
                |bcb| {
                    let mem_segment_type_prev_expr = vc.query_advice(mem_segment_type, Rotation::prev());
                    bcb.require_equal(
                        "is_mem_segment_type_ctx && prev.is_mem_segment_type_ctx => mem_segment_type=prev.mem_segment_type",
                        mem_segment_type_prev_expr.clone(),
                        mem_segment_type_expr.clone(),
                    );
                }
            );

            let mem_segment_type_is_active_expr = mem_segment_type_chip.config.value_equals(MemSegmentType::Active, Rotation::cur())(vc);
            let mem_segment_type_is_passive_expr = mem_segment_type_chip.config.value_equals(MemSegmentType::Passive, Rotation::cur())(vc);
            let mem_segment_type_is_active_variadic_expr = mem_segment_type_chip.config.value_equals(MemSegmentType::ActiveVariadic, Rotation::cur())(vc);
            // constraints for is_mem_segment_type{1}=MemSegmentType::Active:
            // is_items_count+ -> item+ (is_mem_segment_type{1} -> is_mem_segment_size_opcode{1} -> is_mem_segment_size+ -> is_block_end{1} -> is_mem_segment_len+ -> is_mem_segment_bytes*)
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_items_count+ -> item+ (is_mem_segment_type{1} ...",
                is_items_count_expr.clone() * mem_segment_type_is_active_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_items_count, is_mem_segment_type],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_type{1} -> is_mem_segment_size_opcode{1}",
                is_mem_segment_type_expr.clone() * mem_segment_type_is_active_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_size_opcode],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_size_opcode{1} -> is_mem_segment_size+",
                is_mem_segment_size_opcode_expr.clone() * mem_segment_type_is_active_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_size],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_size+ -> is_block_end{1}",
                is_mem_segment_size_expr.clone() * mem_segment_type_is_active_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_size, is_block_end],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_block_end{1} -> is_mem_segment_len+",
                is_block_end_expr.clone() * mem_segment_type_is_active_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_len],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_len+ -> is_mem_segment_bytes*",
                is_mem_segment_len_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_len, is_mem_segment_bytes, is_mem_segment_type],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_len+ -> is_mem_segment_bytes*",
                is_mem_segment_bytes_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_bytes, is_mem_segment_type],
            );
            // constraints for is_mem_segment_type{1}=MemSegmentType::Passive:
            // is_items_count+ -> item+ (is_mem_segment_len{1} -> is_mem_segment_bytes*
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_items_count+ -> item+ (is_mem_segment_len{1} ...",
                is_items_count_expr.clone() * mem_segment_type_is_passive_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_items_count, is_mem_segment_len],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_len+ -> is_mem_segment_bytes*",
                is_mem_segment_len_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_len, is_mem_segment_bytes, is_mem_segment_type],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_len+ -> is_mem_segment_bytes*",
                is_mem_segment_bytes_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_bytes, is_mem_segment_type],
            );
            // constraints for is_mem_segment_type{1}=MemSegmentType::ActiveVariadic:
            //  is_items_count+ -> item+ (is_mem_segment_type{1} -> is_mem_index+ -> is_mem_segment_size_opcode{1} -> is_mem_segment_size+ -> is_block_end{1} -> is_mem_segment_len+ -> is_mem_segment_bytes*)
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_items_count+ -> item+ (is_mem_segment_type{1} ...",
                is_items_count_expr.clone() * mem_segment_type_is_active_variadic_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_items_count, is_mem_segment_type],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_type{1} -> is_mem_index+",
                is_mem_segment_type_expr.clone() * mem_segment_type_is_active_variadic_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_memidx],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_index+ -> is_mem_segment_size_opcode{1}",
                is_mem_segment_type_expr.clone() * mem_segment_type_is_active_variadic_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_memidx, is_mem_segment_size_opcode],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_size_opcode{1} -> is_mem_segment_size+",
                is_mem_segment_size_opcode_expr.clone() * mem_segment_type_is_active_variadic_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_size],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_size+ -> is_block_end{1}",
                is_mem_segment_size_expr.clone() * mem_segment_type_is_active_variadic_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_size, is_block_end],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_block_end{1} -> is_mem_segment_len+",
                is_block_end_expr.clone() * mem_segment_type_is_active_variadic_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_len],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_len+ -> is_mem_segment_bytes*",
                is_mem_segment_len_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_len, is_mem_segment_bytes, is_mem_segment_type],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_mem_segment_bytes*",
                is_mem_segment_bytes_expr.clone() * not_q_last_expr.clone(),
                true,
                &[is_mem_segment_bytes, is_mem_segment_type],
            );

            cb.condition(
                is_block_end_expr.clone(),
                |bcb| {
                    bcb.require_equal(
                        "is_block_end -> byte value = WASM_BLOCK_END",
                        byte_val_expr.clone(),
                        WASM_BLOCK_END.expr(),
                    )
                }
            );

            cb.condition(
                is_mem_segment_type_expr.clone(),
                |bcb| {
                    bcb.require_in_set(
                        "is_mem_segment_type -> byte value is correct",
                        byte_val_expr.clone(),
                        vec![
                            MemSegmentType::Active.expr(),
                            MemSegmentType::Passive.expr(),
                            MemSegmentType::ActiveVariadic.expr(),
                        ],
                    )
                }
            );

            cb.condition(
                is_mem_segment_size_opcode_expr.clone(),
                |bcb| {
                    bcb.require_in_set(
                        "is_mem_segment_size_opcode -> byte value is correct",
                        byte_val_expr.clone(),
                        vec![
                            NumericInstruction::I32Const.expr(),
                        ],
                    )
                }
            );

            cb.gate(q_enable_expr.clone())
        });

        let config = WasmDataSectionBodyConfig::<F> {
            q_enable,
            q_first,
            q_last,
            is_items_count,
            is_memidx,
            is_mem_segment_type,
            is_mem_segment_size_opcode,
            is_mem_segment_size,
            is_block_end,
            is_mem_segment_len,
            is_mem_segment_bytes,
            is_mem_segment_type_ctx,
            leb128_chip,
            dynamic_indexes_chip,
            mem_segment_type,
            mem_segment_type_chip,
            _marker: PhantomData,
        };

        config
    }

    pub fn assign(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        offset: usize,
        assign_types: &[AssignType],
        assign_value: u64,
        leb_params: Option<LebParams>,
    ) {
        let q_enable = true;
        debug!(
            "data_section_body: assign at offset {} q_enable {} assign_types {:?} assign_value {} byte_val {:x?}",
            offset,
            q_enable,
            assign_types,
            assign_value,
            wasm_bytecode.bytes[offset],
        );
        region.assign_fixed(
            || format!("assign 'q_enable' val {} at {}", q_enable, offset),
            self.config.q_enable,
            offset,
            || Value::known(F::from(q_enable as u64)),
        ).unwrap();
        assign_types.iter().for_each(|assign_type| {
            if [
                AssignType::IsItemsCount,
                AssignType::IsMemSegmentSize,
                AssignType::IsMemSegmentLen,
            ].contains(&assign_type) {
                let p = leb_params.unwrap();
                self.config.leb128_chip.assign(
                    region,
                    offset,
                    q_enable,
                    p,
                );
            }
            match assign_type {
                AssignType::QFirst => {
                    region.assign_fixed(
                        || format!("assign 'q_first' val {} at {}", assign_value, offset),
                        self.config.q_first,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::QLast => {
                    region.assign_fixed(
                        || format!("assign 'q_last' val {} at {}", assign_value, offset),
                        self.config.q_last,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::IsItemsCount => {
                    region.assign_fixed(
                        || format!("assign 'is_items_count' val {} at {}", assign_value, offset),
                        self.config.is_items_count,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::IsMemSegmentType => {
                    region.assign_fixed(
                        || format!("assign 'is_mem_segment_type' val {} at {}", assign_value, offset),
                        self.config.is_mem_segment_type,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::IsMemSegmentSizeOpcode => {
                    region.assign_fixed(
                        || format!("assign 'is_mem_segment_size_opcode' val {} at {}", assign_value, offset),
                        self.config.is_mem_segment_size_opcode,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::IsMemSegmentSize => {
                    region.assign_fixed(
                        || format!("assign 'is_mem_segment_size' val {} at {}", assign_value, offset),
                        self.config.is_mem_segment_size,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::IsBlockEnd => {
                    region.assign_fixed(
                        || format!("assign 'is_block_end' val {} at {}", assign_value, offset),
                        self.config.is_block_end,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::IsMemSegmentLen => {
                    region.assign_fixed(
                        || format!("assign 'is_mem_segment_len' val {} at {}", assign_value, offset),
                        self.config.is_mem_segment_len,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::IsMemSegmentBytes => {
                    region.assign_fixed(
                        || format!("assign 'is_mem_segment_bytes' val {} at {}", assign_value, offset),
                        self.config.is_mem_segment_bytes,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::IsMemSegmentTypeCtx => {
                    region.assign_fixed(
                        || format!("assign 'is_mem_segment_type_ctx' val {} at {}", assign_value, offset),
                        self.config.is_mem_segment_type_ctx,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::MemSegmentType => {
                    region.assign_advice(
                        || format!("assign 'mem_segment_type' val {} at {}", assign_value, offset),
                        self.config.mem_segment_type,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                    let mem_segment_type: MemSegmentType = (assign_value as u8).try_into().unwrap();
                    self.config.mem_segment_type_chip.assign(
                        region,
                        offset,
                        &mem_segment_type,
                    ).unwrap();
                }
                AssignType::IsMemIndex => {
                    region.assign_fixed(
                        || format!("assign 'is_mem_index' val {} at {}", assign_value, offset),
                        self.config.is_memidx,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
            }
        })
    }

    /// returns sn and leb len
    fn markup_leb_section(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        leb_bytes_offset: usize,
        assign_types: &[AssignType],
    ) -> (u64, usize) {
        let is_signed = false;
        let (sn, last_byte_offset) = leb128_compute_sn(wasm_bytecode.bytes.as_slice(), is_signed, leb_bytes_offset).unwrap();
        let mut sn_recovered_at_pos = 0;
        let last_byte_rel_offset = last_byte_offset - leb_bytes_offset;
        for byte_rel_offset in 0..=last_byte_rel_offset {
            let offset = leb_bytes_offset + byte_rel_offset;
            sn_recovered_at_pos = leb128_compute_sn_recovered_at_position(
                sn_recovered_at_pos,
                is_signed,
                byte_rel_offset,
                last_byte_rel_offset,
                wasm_bytecode.bytes[offset],
            );
            self.assign(
                region,
                wasm_bytecode,
                offset,
                assign_types,
                1,
                Some(LebParams {
                    is_signed,
                    byte_rel_offset,
                    last_byte_rel_offset,
                    sn,
                    sn_recovered_at_pos,
                }),
            );
        }

        (sn, last_byte_rel_offset + 1)
    }

    /// returns new offset
    fn markup_bytes_section(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        offset: usize,
        len: usize,
    ) -> usize {
        for rel_offset in 0..len {
            self.assign(
                region,
                wasm_bytecode,
                offset + rel_offset,
                &[AssignType::IsMemSegmentBytes],
                1,
                None,
            );
        }
        offset + len
    }

    /// returns new offset
    pub fn assign_auto(
        &self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
        offset_start: usize,
        shared_state: &mut SharedState,
    ) -> Result<usize, Error> {
        let mut offset = offset_start;

        // items_count+
        self.assign(region, &wasm_bytecode, offset, &[AssignType::QFirst], 1, None);
        let (items_count, items_count_leb_len) = self.markup_leb_section(
            region,
            wasm_bytecode,
            offset,
            &[AssignType::IsItemsCount],
        );
        shared_state.dynamic_indexes_offset = self.config.dynamic_indexes_chip.assign_auto(
            region,
            shared_state.dynamic_indexes_offset,
            items_count as usize,
            Tag::DataSectionDataIndex,
        ).unwrap();
        offset += items_count_leb_len;

        for _item_index in 0..items_count {
            // is_mem_segment_type{1}
            let mem_segment_type_val = wasm_bytecode.bytes[offset];
            let mem_segment_type: MemSegmentType = mem_segment_type_val.try_into().unwrap();
            self.assign(
                region,
                wasm_bytecode,
                offset,
                &[AssignType::IsMemSegmentType, AssignType::IsMemSegmentTypeCtx],
                1,
                None,
            );
            self.assign(
                region,
                wasm_bytecode,
                offset,
                &[AssignType::MemSegmentType],
                mem_segment_type_val as u64,
                None,
            );
            offset += 1;

            match mem_segment_type {
                MemSegmentType::Active => {
                    // is_mem_segment_size_opcode{1}
                    self.assign(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsMemSegmentSizeOpcode, AssignType::IsMemSegmentTypeCtx],
                        1,
                        None,
                    );
                    self.assign(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::MemSegmentType],
                        mem_segment_type_val as u64,
                        None,
                    );
                    offset += 1;

                    // is_mem_segment_size+
                    let (mem_segment_size, mem_segment_size_leb_len) = self.markup_leb_section(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsMemSegmentSize, AssignType::IsMemSegmentTypeCtx],
                    );
                    for offset in offset..offset + mem_segment_size_leb_len {
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset,
                            &[AssignType::MemSegmentType],
                            mem_segment_type_val as u64,
                            None,
                        );
                    }
                    offset += mem_segment_size_leb_len;

                    // is_block_end{1}
                    self.assign(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsBlockEnd, AssignType::IsMemSegmentTypeCtx],
                        1,
                        None,
                    );
                    self.assign(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::MemSegmentType],
                        mem_segment_type_val as u64,
                        None,
                    );
                    offset += 1;

                    // is_mem_segment_len+
                    let (mem_segment_len, mem_segment_len_leb_len) = self.markup_leb_section(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsMemSegmentLen, AssignType::IsMemSegmentTypeCtx],
                    );
                    for offset in offset..offset + mem_segment_len_leb_len {
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset,
                            &[AssignType::MemSegmentType],
                            mem_segment_type_val as u64,
                            None,
                        );
                    }
                    offset += mem_segment_len_leb_len;

                    // is_mem_segment_bytes*
                    for rel_offset in 0..(mem_segment_len as usize) {
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset + rel_offset,
                            &[AssignType::IsMemSegmentBytes, AssignType::IsMemSegmentTypeCtx],
                            1,
                            None,
                        );
                        for offset in offset..offset + mem_segment_len_leb_len {
                            self.assign(
                                region,
                                wasm_bytecode,
                                offset,
                                &[AssignType::MemSegmentType],
                                mem_segment_type_val as u64,
                                None,
                            );
                        }
                    }
                    offset += mem_segment_len as usize;
                }
                MemSegmentType::Passive => {
                    // is_mem_segment_len+
                    let (mem_segment_len, mem_segment_len_leb_len) = self.markup_leb_section(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsMemSegmentLen, AssignType::IsMemSegmentTypeCtx],
                    );
                    for offset in offset..offset + mem_segment_len_leb_len {
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset,
                            &[AssignType::MemSegmentType],
                            mem_segment_type_val as u64,
                            None,
                        );
                    }
                    offset += mem_segment_len_leb_len;

                    // is_mem_segment_bytes*
                    for rel_offset in 0..(mem_segment_len as usize) {
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset + rel_offset,
                            &[AssignType::IsMemSegmentBytes, AssignType::IsMemSegmentTypeCtx],
                            1,
                            None,
                        );
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset,
                            &[AssignType::MemSegmentType],
                            mem_segment_type_val as u64,
                            None,
                        );
                    }
                    offset += mem_segment_len as usize;
                }
                MemSegmentType::ActiveVariadic => {
                    // is_mem_index+
                    let (mem_index, mem_index_leb_len) = self.markup_leb_section(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsMemIndex, AssignType::IsMemSegmentTypeCtx],
                    );
                    for offset in offset..offset + mem_index_leb_len {
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset,
                            &[AssignType::MemSegmentType],
                            mem_segment_type_val as u64,
                            None,
                        );
                    }
                    offset += mem_index_leb_len;

                    // is_mem_segment_size_opcode{1}
                    self.assign(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsMemSegmentSizeOpcode, AssignType::IsMemSegmentTypeCtx],
                        1,
                        None,
                    );
                    self.assign(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::MemSegmentType],
                        mem_segment_type_val as u64,
                        None,
                    );
                    offset += 1;

                    // is_mem_segment_size+
                    let (mem_segment_size, mem_segment_size_leb_len) = self.markup_leb_section(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsMemSegmentSize, AssignType::IsMemSegmentTypeCtx],
                    );
                    for offset in offset..offset + mem_segment_size_leb_len {
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset,
                            &[AssignType::MemSegmentType],
                            mem_segment_type_val as u64,
                            None,
                        );
                    }
                    offset += mem_segment_size_leb_len;

                    // is_block_end{1}
                    self.assign(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsBlockEnd, AssignType::IsMemSegmentTypeCtx],
                        1,
                        None,
                    );
                    self.assign(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::MemSegmentType],
                        mem_segment_type_val as u64,
                        None,
                    );
                    offset += 1;

                    // is_mem_segment_len+
                    let (mem_segment_len, mem_segment_len_leb_len) = self.markup_leb_section(
                        region,
                        wasm_bytecode,
                        offset,
                        &[AssignType::IsMemSegmentLen, AssignType::IsMemSegmentTypeCtx],
                    );
                    for offset in offset..offset + mem_segment_len_leb_len {
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset,
                            &[AssignType::MemSegmentType],
                            mem_segment_type_val as u64,
                            None,
                        );
                    }
                    offset += mem_segment_len_leb_len;

                    // is_mem_segment_bytes*
                    for rel_offset in 0..(mem_segment_len as usize) {
                        self.assign(
                            region,
                            wasm_bytecode,
                            offset + rel_offset,
                            &[AssignType::IsMemSegmentBytes, AssignType::IsMemSegmentTypeCtx],
                            1,
                            None,
                        );
                        for offset in offset..offset + mem_segment_len_leb_len {
                            self.assign(
                                region,
                                wasm_bytecode,
                                offset,
                                &[AssignType::MemSegmentType],
                                mem_segment_type_val as u64,
                                None,
                            );
                        }
                    }
                    offset += mem_segment_len as usize;
                }
            }
        }

        if offset != offset_start {
            self.assign(region, &wasm_bytecode, offset - 1, &[AssignType::QLast], 1, None);
        }

        Ok(offset)
    }
}