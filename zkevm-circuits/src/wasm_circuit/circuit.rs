use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use halo2_proofs::{
    plonk::{Column, ConstraintSystem},
};
use halo2_proofs::circuit::{Chip, Layouter, Region, Value};
use halo2_proofs::plonk::{Advice, Constraints, Error, Fixed};
use halo2_proofs::poly::Rotation;
use log::debug;

use eth_types::Field;
use gadgets::is_zero::{IsZeroChip, IsZeroInstruction};
use gadgets::less_than::{LtChip, LtInstruction};
use gadgets::util::{and, Expr, not, or};

use crate::evm_circuit::util::constraint_builder::{BaseConstraintBuilder, ConstrainBuilderCommon};
use crate::table::PoseidonTable;
use crate::wasm_circuit::bytecode::bytecode::WasmBytecode;
use crate::wasm_circuit::bytecode::bytecode_table::WasmBytecodeTable;
use crate::wasm_circuit::common::{wasm_compute_section_len, WasmAssignAwareChip, WasmFuncCountAwareChip, WasmLenPrefixedBytesSpanAwareChip, WasmMarkupLeb128SectionAwareChip, WasmSharedStateAwareChip};
use crate::wasm_circuit::common::configure_transition_check;
use crate::wasm_circuit::consts::{ControlInstruction, ExportDescType, ImportDescType, SECTION_ID_DEFAULT, WASM_PREAMBLE_MAGIC_PREFIX, WASM_SECTION_ID_MAX, WASM_SECTIONS_START_INDEX, WASM_VERSION_PREFIX_BASE_INDEX, WASM_VERSION_PREFIX_LENGTH, WasmSection};
use crate::wasm_circuit::leb128_circuit::circuit::LEB128Chip;
use crate::wasm_circuit::leb128_circuit::helpers::leb128_compute_last_byte_offset;
use crate::wasm_circuit::sections::code::body::circuit::WasmCodeSectionBodyChip;
use crate::wasm_circuit::sections::consts::LebParams;
use crate::wasm_circuit::sections::data::body::circuit::WasmDataSectionBodyChip;
use crate::wasm_circuit::sections::element::body::circuit::WasmElementSectionBodyChip;
use crate::wasm_circuit::sections::export::body::circuit::WasmExportSectionBodyChip;
use crate::wasm_circuit::sections::function::body::circuit::WasmFunctionSectionBodyChip;
use crate::wasm_circuit::sections::global::body::circuit::WasmGlobalSectionBodyChip;
use crate::wasm_circuit::sections::import::body::circuit::WasmImportSectionBodyChip;
use crate::wasm_circuit::sections::memory::body::circuit::WasmMemorySectionBodyChip;
use crate::wasm_circuit::sections::r#type::body::circuit::WasmTypeSectionBodyChip;
use crate::wasm_circuit::sections::r#type::item::circuit::WasmTypeSectionItemChip;
use crate::wasm_circuit::sections::start::body::circuit::WasmStartSectionBodyChip;
use crate::wasm_circuit::sections::table::body::circuit::WasmTableSectionBodyChip;
use crate::wasm_circuit::tables::dynamic_indexes::circuit::DynamicIndexesChip;
use crate::wasm_circuit::tables::dynamic_indexes::types::{LookupArgsParams, Tag};
use crate::wasm_circuit::tables::fixed_range::config::RangeTableConfig;
use crate::wasm_circuit::types::{AssignType, SharedState};
use crate::wasm_circuit::utf8_circuit::circuit::UTF8Chip;

pub struct WasmSectionConfig<F: Field> {
    _marker: PhantomData<F>,
}

#[derive(Debug, Clone)]
pub struct WasmConfig<F: Field> {
    q_enable: Column<Fixed>,
    q_first: Column<Fixed>,
    q_last: Column<Fixed>,
    is_section_id: Column<Fixed>,
    is_section_len: Column<Fixed>,
    is_section_body: Column<Fixed>,

    section_id: Column<Advice>,

    leb128_chip: Rc<LEB128Chip<F>>,
    utf8_chip: Rc<UTF8Chip<F>>,
    wasm_type_section_item_chip: Rc<WasmTypeSectionItemChip<F>>,
    wasm_type_section_body_chip: Rc<WasmTypeSectionBodyChip<F>>,
    wasm_import_section_body_chip: Rc<WasmImportSectionBodyChip<F>>,
    wasm_function_section_body_chip: Rc<WasmFunctionSectionBodyChip<F>>,
    wasm_memory_section_body_chip: Rc<WasmMemorySectionBodyChip<F>>,
    wasm_export_section_body_chip: Rc<WasmExportSectionBodyChip<F>>,
    wasm_data_section_body_chip: Rc<WasmDataSectionBodyChip<F>>,
    wasm_global_section_body_chip: Rc<WasmGlobalSectionBodyChip<F>>,
    wasm_code_section_body_chip: Rc<WasmCodeSectionBodyChip<F>>,
    wasm_start_section_body_chip: Rc<WasmStartSectionBodyChip<F>>,
    wasm_table_section_body_chip: Rc<WasmTableSectionBodyChip<F>>,
    wasm_element_section_body_chip: Rc<WasmElementSectionBodyChip<F>>,
    section_id_lt_chip: LtChip<F, 1>,
    dynamic_indexes_chip: Rc<DynamicIndexesChip<F>>,
    index_at_magic_prefix_count: usize,
    index_at_magic_prefix: Vec<IsZeroChip<F>>,
    index_at_magic_prefix_prev: Vec<IsZeroChip<F>>,
    poseidon_table: PoseidonTable,
    range_table_config_0_256: RangeTableConfig<F, 0, 256>,
    section_id_range_table_config: RangeTableConfig<F, 0, { WASM_SECTION_ID_MAX + 1 }>,
    range_table_config_0_128: Rc<RangeTableConfig<F, 0, 128>>,
    wasm_bytecode_table: Rc<WasmBytecodeTable>,

    func_count: Column<Advice>,
    block_depth_level: Column<Advice>,
    body_byte_rev_index_l1: Column<Advice>,
    body_byte_rev_index_l2: Column<Advice>,
    body_item_rev_count_l1: Column<Advice>,
    body_item_rev_count_l2: Column<Advice>,

    pub shared_state: Rc<RefCell<SharedState>>,

    _marker: PhantomData<F>,
}

impl<F: Field> WasmConfig<F> {}


#[derive(Debug, Clone)]
pub struct WasmChip<F: Field> {
    pub config: WasmConfig<F>,
    _marker: PhantomData<F>,
}

impl<F: Field> WasmAssignAwareChip<F> for WasmChip<F> {
    type AssignType = AssignType;

    fn assign(
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
            "assign at offset {} q_enable {} assign_types {:?} assign_value {} byte_val {:x?}",
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
            match assign_type {
                AssignType::Unknown => {
                    panic!("unknown assign type")
                }
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
                AssignType::IsSectionId => {
                    region.assign_fixed(
                        || format!("assign 'is_section_id' val {} at {}", assign_value, offset),
                        self.config.is_section_id,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::IsSectionLen => {
                    region.assign_fixed(
                        || format!("assign 'is_section_len' val {} at {}", assign_value, offset),
                        self.config.is_section_len,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                    let p = leb_params.unwrap();
                    self.config.leb128_chip.assign(
                        region,
                        offset,
                        q_enable,
                        p,
                    );
                }
                AssignType::IsSectionBody => {
                    region.assign_fixed(
                        || format!("assign 'is_section_body' val {} at {}", assign_value, offset),
                        self.config.is_section_body,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
                AssignType::BodyByteRevIndexL1 => {
                    region.assign_advice(
                        || format!("assign 'body_byte_rev_index_l1' val {} at {}", assign_value, offset),
                        self.config.body_byte_rev_index_l1,
                        offset,
                        || Value::known(F::from(assign_value)),
                    ).unwrap();
                }
            }

            for (index, index_at_magic_prefix) in self.config.index_at_magic_prefix.iter().enumerate() {
                index_at_magic_prefix.assign(region, offset, Value::known(F::from(offset as u64) - F::from(index as u64))).unwrap();
            }
            for (index, index_at_magic_prefix_prev) in self.config.index_at_magic_prefix_prev.iter().enumerate() {
                index_at_magic_prefix_prev.assign(region, offset, Value::known(F::from(offset as u64) - F::from(index as u64) - F::from(1))).unwrap();
            }
        });
    }
}

impl<F: Field> WasmLenPrefixedBytesSpanAwareChip<F> for WasmChip<F> {}

impl<F: Field> WasmMarkupLeb128SectionAwareChip<F> for WasmChip<F> {}

impl<F: Field> WasmSharedStateAwareChip<F> for WasmChip<F> {
    fn shared_state(&self) -> Rc<RefCell<SharedState>> { self.config.shared_state.clone() }
}

impl<F: Field> WasmFuncCountAwareChip<F> for WasmChip<F> {
    fn func_count_col(&self) -> Column<Advice> { self.config.func_count }
}

impl<F: Field> WasmChip<F>
{
    pub fn load(
        &self,
        layouter: &mut impl Layouter<F>,
        wasm_bytecode: &WasmBytecode,
    ) -> Result<(), Error> {
        self.config.wasm_bytecode_table.load(layouter, wasm_bytecode)?;
        self.config.range_table_config_0_256.load(layouter)?;
        self.config.section_id_range_table_config.load(layouter)?;
        self.config.range_table_config_0_128.load(layouter)?;

        self.config
            .poseidon_table
            .dev_load(layouter, &[wasm_bytecode.bytes.clone()])?;

        Ok(())
    }

    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        wasm_bytecode_table: Rc<WasmBytecodeTable>,
        shared_state: Rc<RefCell<SharedState>>,
    ) -> WasmConfig<F> {
        let index_at_magic_prefix_count = WASM_PREAMBLE_MAGIC_PREFIX.len() + WASM_VERSION_PREFIX_LENGTH;

        let q_enable = cs.fixed_column();
        let q_first = cs.fixed_column();
        let q_last = cs.fixed_column();
        let is_section_id = cs.fixed_column();
        let is_section_len = cs.fixed_column();
        let is_section_body = cs.fixed_column();

        let section_id = cs.advice_column();
        let func_count = cs.advice_column();
        let block_depth_level = cs.advice_column();
        let body_byte_rev_index_l1 = cs.advice_column();
        let body_byte_rev_index_l2 = cs.advice_column();
        let body_item_rev_count_l1 = cs.advice_column();
        let body_item_rev_count_l2 = cs.advice_column();

        let range_table_config_0_256 = RangeTableConfig::configure(cs);
        let section_id_range_table_config = RangeTableConfig::configure(cs);
        let range_table_config_0_128 = Rc::new(RangeTableConfig::configure(cs));
        let poseidon_table = PoseidonTable::dev_construct(cs);

        let leb128_config = LEB128Chip::configure(
            cs,
            &wasm_bytecode_table.value,
        );
        let mut leb128_chip = Rc::new(LEB128Chip::construct(leb128_config));

        let utf8_config = UTF8Chip::configure(
            cs,
            range_table_config_0_128.clone(),
            &wasm_bytecode_table.value,
        );
        let mut utf8_chip = Rc::new(UTF8Chip::construct(utf8_config));

        let config = DynamicIndexesChip::configure(cs);
        let dynamic_indexes_chip = Rc::new(DynamicIndexesChip::construct(config));

        let config = WasmTypeSectionItemChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            func_count,
            shared_state.clone(),
            body_item_rev_count_l2,
        );
        let wasm_type_section_item_chip = Rc::new(WasmTypeSectionItemChip::construct(config));
        let config = WasmTypeSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            wasm_type_section_item_chip.clone(),
            dynamic_indexes_chip.clone(),
            func_count,
            shared_state.clone(),
            body_item_rev_count_l1,
        );
        let wasm_type_section_body_chip = Rc::new(WasmTypeSectionBodyChip::construct(config));

        let config = WasmImportSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            utf8_chip.clone(),
            dynamic_indexes_chip.clone(),
            func_count,
            shared_state.clone(),
            body_byte_rev_index_l2,
            body_item_rev_count_l1,
        );
        let wasm_import_section_body_chip = Rc::new(WasmImportSectionBodyChip::construct(config));

        let config = WasmFunctionSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            func_count,
            shared_state.clone(),
            body_item_rev_count_l1,
        );
        let wasm_function_section_body_chip = Rc::new(WasmFunctionSectionBodyChip::construct(config));

        let config = WasmMemorySectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            dynamic_indexes_chip.clone(),
            func_count,
            shared_state.clone(),
            body_item_rev_count_l1,
        );
        let wasm_memory_section_body_chip = Rc::new(WasmMemorySectionBodyChip::construct(config));

        let config = WasmExportSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            func_count,
            shared_state.clone(),
            body_byte_rev_index_l2,
            body_item_rev_count_l1,
        );
        let wasm_export_section_body_chip = Rc::new(WasmExportSectionBodyChip::construct(config));

        let config = WasmDataSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            dynamic_indexes_chip.clone(),
            func_count,
            shared_state.clone(),
            body_byte_rev_index_l2,
            body_item_rev_count_l1,
        );
        let wasm_data_section_body_chip = Rc::new(WasmDataSectionBodyChip::construct(config));

        let config = WasmGlobalSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            dynamic_indexes_chip.clone(),
            func_count,
            shared_state.clone(),
            body_item_rev_count_l1,
        );
        let wasm_global_section_body_chip = Rc::new(WasmGlobalSectionBodyChip::construct(config));

        let config = WasmCodeSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            dynamic_indexes_chip.clone(),
            func_count,
            shared_state.clone(),
            body_byte_rev_index_l2,
            body_item_rev_count_l1,
        );
        let wasm_code_section_body_chip = Rc::new(WasmCodeSectionBodyChip::construct(config));

        let config = WasmStartSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            func_count,
            shared_state.clone(),
        );
        let wasm_start_section_body_chip = Rc::new(WasmStartSectionBodyChip::construct(config));

        let config = WasmElementSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            func_count,
            shared_state.clone(),
            body_item_rev_count_l1,
        );
        let wasm_element_section_body_chip = Rc::new(WasmElementSectionBodyChip::construct(config));

        let config = WasmTableSectionBodyChip::configure(
            cs,
            wasm_bytecode_table.clone(),
            leb128_chip.clone(),
            dynamic_indexes_chip.clone(),
            func_count,
            shared_state.clone(),
        );
        let wasm_table_section_body_chip = Rc::new(WasmTableSectionBodyChip::construct(config));

        let mut index_at_magic_prefix: Vec<IsZeroChip<F>> = Vec::new();
        for index in 0..index_at_magic_prefix_count {
            let value_inv = cs.advice_column();
            let index_at_magic_prefix_config = IsZeroChip::configure(
                cs,
                |vc| vc.query_fixed(q_enable, Rotation::cur()),
                |vc| vc.query_advice(wasm_bytecode_table.index, Rotation::cur()) - index.expr(),
                value_inv
            );
            let chip = IsZeroChip::construct(index_at_magic_prefix_config);
            index_at_magic_prefix.push(chip);
        }
        let mut index_at_magic_prefix_prev: Vec<IsZeroChip<F>> = Vec::new();
        for index in 0..index_at_magic_prefix_count {
            let value_inv = cs.advice_column();
            let index_at_magic_prefix_prev_config = IsZeroChip::configure(
                cs,
                |vc| and::expr([vc.query_fixed(q_enable, Rotation::cur()), not::expr(vc.query_fixed(q_first, Rotation::cur()))]),
                |vc| vc.query_advice(wasm_bytecode_table.index, Rotation::prev()) - index.expr(),
                value_inv
            );
            let chip = IsZeroChip::construct(index_at_magic_prefix_prev_config);
            index_at_magic_prefix_prev.push(chip);
        }

        Self::configure_len_prefixed_bytes_span_checks(
            cs,
            leb128_chip.as_ref(),
            |vc| { vc.query_fixed(is_section_body, Rotation::cur()) },
            body_byte_rev_index_l1,
            |vc| {
                let not_q_last_expr = not::expr(vc.query_fixed(q_last, Rotation::cur()));
                let is_section_len_expr = vc.query_fixed(is_section_len, Rotation::cur());
                let is_section_body_next_expr = vc.query_fixed(is_section_body, Rotation::next());

                and::expr([not_q_last_expr, is_section_len_expr, is_section_body_next_expr])
            },
            |vc| {
                let q_last_expr = vc.query_fixed(q_last, Rotation::cur());
                let is_section_body_expr = vc.query_fixed(is_section_body, Rotation::cur());
                let is_section_id_next_expr = vc.query_fixed(is_section_id, Rotation::next());

                or::expr([
                    q_last_expr,
                    and::expr([
                        is_section_body_expr,
                        is_section_id_next_expr,
                    ])
                ])
            },
        );

        cs.create_gate("basic row checks", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_enable_expr = vc.query_fixed(q_enable, Rotation::cur());
            let q_first_expr = vc.query_fixed(q_first, Rotation::cur());
            let q_last_expr = vc.query_fixed(q_last, Rotation::cur());

            let is_section_id_expr = vc.query_fixed(is_section_id, Rotation::cur());
            let is_section_len_expr = vc.query_fixed(is_section_len, Rotation::cur());
            let is_section_body_expr = vc.query_fixed(is_section_body, Rotation::cur());

            let byte_val_expr = vc.query_advice(wasm_bytecode_table.index, Rotation::cur());

            cb.require_boolean("q_enable is boolean", q_enable_expr.clone());
            cb.require_boolean("q_first is boolean", q_first_expr.clone());
            cb.require_boolean("q_last is boolean", q_last_expr.clone());
            cb.require_boolean("is_section_id is boolean", is_section_id_expr.clone());
            cb.require_boolean("is_section_len is boolean", is_section_len_expr.clone());
            cb.require_boolean("is_section_body is boolean", is_section_body_expr.clone());

            cb.require_zero("index=0 => q_first=1", and::expr([q_first_expr.clone(), byte_val_expr.clone(), ]));

            let mut is_index_at_magic_prefix_expr = index_at_magic_prefix.iter()
                .fold(0.expr(), |acc, x| { acc.clone() + x.config().expr() });

            // TODO re-check
            cb.require_equal(
                "exactly one mark flag active at the same time",
                is_index_at_magic_prefix_expr.clone()
                    + is_section_id_expr.clone()
                    + is_section_len_expr.clone()
                    + is_section_body_expr.clone(),
                1.expr(),
            );

            cb.gate(q_enable_expr)
        });

        cs.create_gate("bytecode checks", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_enable_expr = vc.query_fixed(q_enable, Rotation::cur());
            let q_last_expr = vc.query_fixed(q_last, Rotation::cur());

            let bytecode_index_expr = vc.query_advice(wasm_bytecode_table.index, Rotation::cur());
            let bytecode_index_next_expr = vc.query_advice(wasm_bytecode_table.index, Rotation::next());

            cb.require_equal(
                "next.bytecode_index = cur.bytecode_index + 1",
                bytecode_index_expr.clone() + 1.expr(),
                bytecode_index_next_expr.clone(),
            );

            cb.gate(and::expr(vec![
                q_enable_expr.clone(),
                not::expr(q_last_expr.clone()),
            ]))
        });
        cs.lookup("all bytecode values are byte values", |vc| {
            let bytecode_value_expr = vc.query_advice(wasm_bytecode_table.value, Rotation::cur());

            vec![(bytecode_value_expr, range_table_config_0_256.value)]
        });

        cs.create_gate("wasm magic prefix check", |vc| {
            let mut cb = BaseConstraintBuilder::default();
            let bytecode_value = vc.query_advice(wasm_bytecode_table.value, Rotation::cur());

            for (i, char) in WASM_PREAMBLE_MAGIC_PREFIX.chars().enumerate() {
                cb.require_zero(
                    "bytecode_val=ord(specific_char) at index",
                    and::expr([
                        index_at_magic_prefix[i].config().expr(),
                        bytecode_value.clone() - (char as i32).expr(),
                    ])
                );
            }
            for i in WASM_VERSION_PREFIX_BASE_INDEX..WASM_VERSION_PREFIX_BASE_INDEX + WASM_VERSION_PREFIX_LENGTH {
                let version_val = if i == WASM_VERSION_PREFIX_BASE_INDEX { 1 } else { 0 };
                cb.require_zero(
                    "bytecode_val=version_val at index",
                    and::expr([
                        index_at_magic_prefix[i].config().expr(),
                        vc.query_advice(wasm_bytecode_table.value, Rotation::cur()) - version_val.expr(),
                    ])
                );
            }

            cb.gate(and::expr(vec![
                vc.query_fixed(q_enable, Rotation::cur()),
            ]))
        });

        cs.create_gate("wasm magic prefix to sections transition check", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_first_expr = vc.query_fixed(q_first, Rotation::cur());
            let func_count_expr = vc.query_advice(func_count, Rotation::cur());

            let q_enable_expr = vc.query_fixed(q_enable, Rotation::cur());
            let is_section_id_expr = vc.query_fixed(is_section_id, Rotation::cur());
            let is_section_len_expr = vc.query_fixed(is_section_len, Rotation::cur());
            let is_section_body_expr = vc.query_fixed(is_section_body, Rotation::cur());

            let mut is_index_at_magic_prefix_expr = index_at_magic_prefix.iter()
                .fold(0.expr(), |acc, x| { acc.clone() + x.config().expr() });

            cb.condition(
                is_index_at_magic_prefix_expr.clone(),
                |bcb| {
                    bcb.require_zero(
                        "bytecode[0..7] -> !is_section_id && !is_section_len && !is_section_body",
                        or::expr([
                            is_section_id_expr.clone(),
                            is_section_len_expr.clone(),
                            is_section_body_expr.clone(),
                        ]),
                    )
                }
            );
            cb.condition(
                not::expr(is_index_at_magic_prefix_expr.clone()),
                |bcb| {
                    bcb.require_equal(
                        "not(bytecode[0..7]) -> one_of([is_section_id, is_section_len, is_section_body])=1",
                        is_section_id_expr.clone() + is_section_len_expr.clone() + is_section_body_expr.clone(),
                        1.expr(),
                    )
                }
            );

            cb.condition(
                is_section_body_expr.clone(),
                |bcb| {
                    bcb.require_equal(
                        "is_section_body -> exactly one section chip is enabled",
                        vc.query_fixed(wasm_type_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_import_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_function_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_memory_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_export_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_data_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_global_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_code_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_start_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_table_section_body_chip.config.q_enable, Rotation::cur())
                            + vc.query_fixed(wasm_element_section_body_chip.config.q_enable, Rotation::cur())
                            + is_section_id_expr.clone()
                            + is_section_len_expr.clone()
                        ,
                        1.expr(),
                    );
                }
            );

            // func_count constraints
            cb.condition(
                q_first_expr.clone(),
                |bcb| {
                    bcb.require_zero(
                        "q_first => func_count=0",
                        func_count_expr.clone(),
                    );
                }
            );
            let importdesc_type_is_typeidx_expr = and::expr([
                vc.query_fixed(wasm_import_section_body_chip.config.is_importdesc_type, Rotation::cur()),
                wasm_import_section_body_chip.config.importdesc_type_chip.config.value_equals(ImportDescType::Typeidx, Rotation::cur())(vc),
            ]);
            let wasm_code_section_q_first_expr = vc.query_fixed(wasm_code_section_body_chip.config.q_first, Rotation::cur());
            let not_func_count_inc_expr = and::expr([
                not::expr(importdesc_type_is_typeidx_expr.clone()),
                not::expr(wasm_code_section_q_first_expr.clone()),
            ]);
            cb.condition(
                and::expr([
                    not::expr(q_first_expr.clone()),
                    not_func_count_inc_expr.clone(),
                ]),
                |bcb| {
                    let func_count_prev_expr = vc.query_advice(func_count, Rotation::prev());
                    bcb.require_equal(
                        "not_q_first && not_func_count_inc => prev.func_count=func_count",
                        func_count_prev_expr.clone(),
                        func_count_expr.clone(),
                    );
                }
            );
            cb.condition(
                importdesc_type_is_typeidx_expr.clone(),
                |bcb| {
                    let func_count_prev_expr = vc.query_advice(func_count, Rotation::prev());
                    bcb.require_equal(
                        "importdesc_type_is_typeidx => func_count increased by 1",
                        func_count_prev_expr.clone() + 1.expr(),
                        func_count_expr.clone(),
                    );
                }
            );
            cb.condition(
                wasm_code_section_q_first_expr.clone(),
                |bcb| {
                    let func_count_prev_expr = vc.query_advice(func_count, Rotation::prev());
                    let wasm_code_section_leb128_sn_expr = vc.query_advice(wasm_code_section_body_chip.config.leb128_chip.config.sn, Rotation::cur());
                    bcb.require_equal(
                        "wasm_code_section_q_first => func_count grew by specific number",
                        func_count_prev_expr.clone() + wasm_code_section_leb128_sn_expr.clone(),
                        func_count_expr.clone(),
                    );
                }
            );

            cb.gate(q_enable_expr.clone())
        });
        cs.create_gate("wasm section layout check", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_first_expr = vc.query_fixed(q_first, Rotation::cur());
            let not_q_first_expr = not::expr(q_first_expr.clone());
            let q_last_expr = vc.query_fixed(q_last, Rotation::cur());
            let not_q_last_expr = not::expr(q_last_expr.clone());

            let bytecode_value = vc.query_advice(wasm_bytecode_table.value, Rotation::cur());

            let section_id_expr = vc.query_advice(section_id, Rotation::cur());

            let is_section_id_expr = vc.query_fixed(is_section_id, Rotation::cur());
            let is_section_len_expr = vc.query_fixed(is_section_len, Rotation::cur());
            let is_section_body_expr = vc.query_fixed(is_section_body, Rotation::cur());

            let leb128_is_last_byte_expr = vc.query_fixed(leb128_chip.config.is_last_byte, Rotation::cur());

            cb.condition(
                index_at_magic_prefix_prev[WASM_SECTIONS_START_INDEX - 1].config().expr(),
                |bcb| {
                    bcb.require_equal(
                        "if previous bytecode index is 7 -> is_section_id",
                        is_section_id_expr.clone(),
                        1.expr(),
                    )
                }
            );
            // section+(is_section_id{1} -> is_section_len+ -> is_section_body+)
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_section_id{1} -> is_section_len+",
                and::expr([
                    not_q_last_expr.clone(),
                    is_section_id_expr.clone(),
                ]),
                true,
                &[is_section_len],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_section_len+ -> is_section_body+",
                and::expr([
                    not_q_last_expr.clone(),
                    is_section_len_expr.clone(),
                ]),
                true,
                &[is_section_len, is_section_body],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next (last leb byte): is_section_len+ -> is_section_body+",
                and::expr([
                    not_q_last_expr.clone(),
                    leb128_is_last_byte_expr.clone(),
                    is_section_len_expr.clone(),
                ]),
                true,
                &[is_section_body],
            );
            configure_transition_check(
                &mut cb,
                vc,
                "check next: is_section_body+ -> is_section_id{1} || q_last",
                and::expr([
                    not_q_last_expr.clone(),
                    is_section_body_expr.clone(),
                ]),
                true,
                &[is_section_body, is_section_id],
            );

            cb.condition(
                is_section_id_expr.clone(),
                |bcb| {
                    bcb.require_equal(
                        "is_section_id -> section_id=bytecode_value",
                        section_id_expr.clone(),
                        bytecode_value.clone(),
                    )
                }
            );

            cb.require_equal(
                "prev.hash = cur.hash",
                vc.query_advice(wasm_bytecode_table.code_hash, Rotation::prev()),
                vc.query_advice(wasm_bytecode_table.code_hash, Rotation::cur()),
            );

            cb.gate(and::expr(vec![
                not::expr(vc.query_fixed(q_first, Rotation::cur())),
                vc.query_fixed(q_enable, Rotation::cur()),
            ]))
        });

        cs.create_gate("for the first 8 bytes section_id=SECTION_ID_DEFAULT", |vc| {
            let section_id_expr = vc.query_advice(section_id, Rotation::cur());

            let mut constraints = Vec::new();
            for i in 0..WASM_SECTIONS_START_INDEX {
                let constraint = index_at_magic_prefix[i].config().expr() * (section_id_expr.clone() - SECTION_ID_DEFAULT.expr());
                constraints.push(
                    ("id of section equals to default at magic prefix indexes", constraint)
                );
            }
            Constraints::with_selector(
                vc.query_fixed(q_enable, Rotation::cur()),
                constraints,
            )
        });

        let section_id_lt_chip_config = LtChip::configure(
            cs,
            |vc| {
                let q_enable_expr = vc.query_fixed(q_enable, Rotation::cur());
                let q_first_expr = vc.query_fixed(q_first, Rotation::cur());
                let not_q_first_expr = not::expr(q_first_expr.clone());

                and::expr([
                    not_q_first_expr.clone(),
                    q_enable_expr.clone(),
                ])
            },
            |vc| {
                vc.query_advice(section_id, Rotation::prev())
            },
            |vc| {
                vc.query_advice(section_id, Rotation::cur())
            },
        );
        let section_id_lt_chip = LtChip::construct(section_id_lt_chip_config);
        cs.create_gate("prev.section_id <= cur.section_id", |vc| {
            let section_id_prev_expr = vc.query_advice(section_id, Rotation::prev());
            let section_id_expr = vc.query_advice(section_id, Rotation::cur());

            let mut constraints = Vec::new();

            constraints.push(
                ("prev.section_id <= cur.section_id",
                 (section_id_lt_chip.config().is_lt(vc, None) - 1.expr())
                     * (section_id_expr.clone() - section_id_prev_expr.clone())
                )
            );

            Constraints::with_selector(
                and::expr([
                    not::expr(vc.query_fixed(q_first, Rotation::cur())),
                    vc.query_fixed(q_enable, Rotation::cur()),
                ]),
                constraints,
            )
        });

        cs.create_gate("code_hash check", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let wasm_bytecode_table_code_hash = vc.query_advice(wasm_bytecode_table.code_hash, Rotation::cur());
            let poseidon_table_hash_id = vc.query_advice(poseidon_table.hash_id, Rotation::cur());

            cb.require_zero(
                "code hashes match",
                wasm_bytecode_table_code_hash.clone() - poseidon_table_hash_id.clone(),
            );

            cb.gate(
                and::expr([
                    index_at_magic_prefix[2].config().expr(),
                    vc.query_fixed(q_enable, Rotation::cur()),
                ]),
            )
        });

        cs.lookup("section_id is a valid number", |vc| {
            let section_id_expr = vc.query_advice(section_id, Rotation::cur());

            vec![(section_id_expr.clone(), section_id_range_table_config.value)]
        });

        // start section crosschecks
        dynamic_indexes_chip.lookup_args(
            "start section: func index refs are valid",
            cs,
            |vc| {
                let sn_expr = vc.query_advice(wasm_start_section_body_chip.config.leb128_chip.config.sn, Rotation::cur());
                LookupArgsParams {
                    cond: vc.query_fixed(wasm_start_section_body_chip.config.is_func_index, Rotation::cur()),
                    index: sn_expr.clone(),
                    tag: Tag::FuncIndex.expr(),
                    is_terminator: false.expr(),
                }
            }
        );
        // import section crosschecks
        dynamic_indexes_chip.lookup_args(
            "import section: typeidx refs are valid",
            cs,
            |vc| {
                let cond = and::expr([
                    vc.query_fixed(wasm_import_section_body_chip.config.is_importdesc_type, Rotation::cur()),
                    wasm_import_section_body_chip.config.importdesc_type_chip.config.value_equals(ImportDescType::Typeidx, Rotation::cur())(vc),
                ]);

                LookupArgsParams {
                    cond,
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::next()),
                    tag: Tag::TypeIndex.expr(),
                    is_terminator: false.expr(),
                }
            }
        );
        // export section crosschecks
        dynamic_indexes_chip.lookup_args(
            "export section: funcidx refs are valid",
            cs,
            |vc| {
                let cond = and::expr([
                    vc.query_fixed(wasm_export_section_body_chip.config.is_exportdesc_type, Rotation::cur()),
                    wasm_export_section_body_chip.config.exportdesc_type_chip.config.value_equals(ExportDescType::Funcidx, Rotation::cur())(vc),
                ]);

                LookupArgsParams {
                    cond,
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::next()),
                    tag: Tag::TypeIndex.expr(),
                    is_terminator: false.expr(),
                }
            }
        );
        dynamic_indexes_chip.lookup_args(
            "export section: tableidx refs are valid",
            cs,
            |vc| {
                let cond = and::expr([
                    vc.query_fixed(wasm_export_section_body_chip.config.is_exportdesc_type, Rotation::cur()),
                    wasm_export_section_body_chip.config.exportdesc_type_chip.config.value_equals(ExportDescType::Tableidx, Rotation::cur())(vc),
                ]);

                LookupArgsParams {
                    cond,
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::next()),
                    tag: Tag::TableIndex.expr(),
                    is_terminator: false.expr(),
                }
            }
        );
        dynamic_indexes_chip.lookup_args(
            "export section: memidx refs are valid",
            cs,
            |vc| {
                let cond = and::expr([
                    vc.query_fixed(wasm_export_section_body_chip.config.is_exportdesc_type, Rotation::cur()),
                    wasm_export_section_body_chip.config.exportdesc_type_chip.config.value_equals(ExportDescType::Memidx, Rotation::cur())(vc),
                ]);

                LookupArgsParams {
                    cond,
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::next()),
                    tag: Tag::MemIndex.expr(),
                    is_terminator: false.expr(),
                }
            }
        );
        dynamic_indexes_chip.lookup_args(
            "export section: globalidx refs are valid",
            cs,
            |vc| {
                let cond = and::expr([
                    vc.query_fixed(wasm_export_section_body_chip.config.is_exportdesc_type, Rotation::cur()),
                    wasm_export_section_body_chip.config.exportdesc_type_chip.config.value_equals(ExportDescType::Globalidx, Rotation::cur())(vc),
                ]);

                LookupArgsParams {
                    cond,
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::next()),
                    tag: Tag::GlobalIndex.expr(),
                    is_terminator: false.expr(),
                }
            }
        );
        // func section crosschecks
        dynamic_indexes_chip.lookup_args(
            "function section: funcidx refs are valid",
            cs,
            |vc| {
                let cond = and::expr([
                    vc.query_fixed(wasm_function_section_body_chip.config.is_typeidx, Rotation::cur()),
                ]);

                LookupArgsParams {
                    cond,
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::next()),
                    tag: Tag::TypeIndex.expr(),
                    is_terminator: false.expr(),
                }
            }
        );
        // data section crosschecks
        dynamic_indexes_chip.lookup_args(
            "data section: memidx refs are valid",
            cs,
            |vc| {
                let cond = and::expr([
                    vc.query_fixed(wasm_data_section_body_chip.config.is_memidx, Rotation::cur()),
                ]);

                LookupArgsParams {
                    cond,
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::next()),
                    tag: Tag::MemIndex.expr(),
                    is_terminator: false.expr(),
                }
            }
        );
        // code section crosschecks
        dynamic_indexes_chip.lookup_args(
            "code section has valid setup for func indexes",
            cs,
            |vc| {
                let q_last_expr = vc.query_fixed(q_last, Rotation::cur());
                LookupArgsParams {
                    cond: q_last_expr,
                    index: vc.query_advice(func_count, Rotation::cur()),
                    tag: Tag::FuncIndex.expr(),
                    is_terminator: true.expr(),
                }
            }
        );
        dynamic_indexes_chip.lookup_args(
            "code section: call opcode param is valid",
            cs,
            |vc| {
                let cond = and::expr([
                    vc.query_fixed(wasm_code_section_body_chip.config.is_control_instruction, Rotation::cur()),
                    wasm_code_section_body_chip.config.control_instruction_chip.config.value_equals(ControlInstruction::Call, Rotation::cur())(vc),
                ]);

                LookupArgsParams {
                    cond,
                    index: vc.query_advice(leb128_chip.config.sn, Rotation::next()),
                    tag: Tag::FuncIndex.expr(),
                    is_terminator: false.expr(),
                }
            }
        );

        let config = WasmConfig {
            _marker: PhantomData,

            poseidon_table,
            wasm_bytecode_table,
            q_enable,
            q_first,
            q_last,
            range_table_config_0_256,
            section_id_range_table_config,
            index_at_magic_prefix,
            index_at_magic_prefix_prev,
            index_at_magic_prefix_count,
            section_id,
            is_section_id,
            is_section_len,
            is_section_body,
            leb128_chip,
            utf8_chip,
            wasm_type_section_item_chip,
            wasm_type_section_body_chip,
            wasm_import_section_body_chip,
            wasm_function_section_body_chip,
            wasm_memory_section_body_chip,
            wasm_export_section_body_chip,
            wasm_data_section_body_chip,
            wasm_global_section_body_chip,
            wasm_code_section_body_chip,
            wasm_start_section_body_chip,
            wasm_table_section_body_chip,
            wasm_element_section_body_chip,
            section_id_lt_chip,
            range_table_config_0_128,
            dynamic_indexes_chip,
            shared_state,
            func_count,
            block_depth_level,
            body_byte_rev_index_l1,
            body_byte_rev_index_l2,
            body_item_rev_count_l1,
            body_item_rev_count_l2,
        };

        config
    }

    pub fn construct(config: WasmConfig<F>) -> Self {
        let instance = Self {
            config,
            _marker: PhantomData,
        };
        instance
    }

    pub fn assign_auto(
        &mut self,
        region: &mut Region<F>,
        wasm_bytecode: &WasmBytecode,
    ) -> Result<(), Error> {
        debug!("wasm_bytecode.bytes {:x?}", wasm_bytecode.bytes);
        self.assign(
            region,
            wasm_bytecode,
            0,
            &[AssignType::QFirst],
            1,
            None,
        );
        self.assign(
            region,
            wasm_bytecode,
            wasm_bytecode.bytes.len() - 1,
            &[AssignType::QLast],
            1,
            None,
        );

        let mut wasm_bytes_offset = WASM_SECTIONS_START_INDEX;
        let mut section_id_prev: i64 = SECTION_ID_DEFAULT as i64;
        loop {
            let section_start_offset = wasm_bytes_offset;
            let section_len_start_offset = section_start_offset + 1;
            let section_id = wasm_bytecode.bytes[wasm_bytes_offset];
            wasm_bytes_offset += 1;
            let (section_len, section_len_leb_bytes_count) = wasm_compute_section_len(&wasm_bytecode.bytes, wasm_bytes_offset).unwrap();
            wasm_bytes_offset += section_len_leb_bytes_count as usize;
            wasm_bytes_offset += section_len;
            let section_body_start_offset = section_len_start_offset + (section_len_leb_bytes_count as usize);
            let section_len_end_offset = section_body_start_offset - 1;
            let section_body_end_offset = section_start_offset + section_len_leb_bytes_count as usize + section_len;
            let section_end_offset = section_body_end_offset;

            for offset in section_start_offset..=section_end_offset {
                if offset == section_start_offset {
                    let wasm_section: WasmSection = (section_id as i32).try_into().unwrap();
                    debug!(
                        "wasm_section {:?}(id={}) at offset {} section_len {} bytecode(hex) {:x?}",
                        wasm_section,
                        section_id,
                        offset,
                        section_len,
                        &wasm_bytecode.bytes[section_start_offset..=section_end_offset],
                    );
                    self.assign_func_count(region, offset);

                    let mut next_section_offset = 0;
                    let section_body_offset = offset + 1; // skip section_id
                    let section_len_last_byte_offset = leb128_compute_last_byte_offset(
                        &wasm_bytecode.bytes[..],
                        section_body_offset,
                    ).unwrap();
                    for offset in section_len_last_byte_offset..=section_body_end_offset {
                        self.assign(
                            region,
                            &wasm_bytecode,
                            offset,
                            &[AssignType::BodyByteRevIndexL1],
                            (section_body_end_offset - offset) as u64,
                            None,
                        );
                    }
                    for offset in section_body_offset..=section_len_last_byte_offset {
                        self.assign_func_count(region, offset);
                    }
                    let section_body_offset = section_len_last_byte_offset + 1;
                    match wasm_section {
                        WasmSection::Type => {
                            next_section_offset = self.config.wasm_type_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Import => {
                            next_section_offset = self.config.wasm_import_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Function => {
                            next_section_offset = self.config.wasm_function_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Table => {
                            next_section_offset = self.config.wasm_table_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Memory => {
                            next_section_offset = self.config.wasm_memory_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Global => {
                            next_section_offset = self.config.wasm_global_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Export => {
                            next_section_offset = self.config.wasm_export_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Start => {
                            next_section_offset = self.config.wasm_start_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Element => {
                            next_section_offset = self.config.wasm_element_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Code => {
                            next_section_offset = self.config.wasm_code_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        WasmSection::Data => {
                            next_section_offset = self.config.wasm_data_section_body_chip.assign_auto(
                                region,
                                wasm_bytecode,
                                section_body_offset,
                            ).unwrap();
                        }
                        _ => { panic!("unsupported section '{:x?}'", wasm_section) }
                    }
                    debug!(
                        "wasm_section {:?} section_body_offset {} after assign_auto next_section_offset {}",
                        wasm_section,
                        section_body_offset,
                        next_section_offset,
                    );
                }
                region.assign_advice(
                    || format!("assign 'section_id' to {} at {}", section_id, offset),
                    self.config.section_id,
                    offset,
                    || Value::known(F::from(section_id as u64))
                )?;
                self.config.section_id_lt_chip.assign(
                    region,
                    offset,
                    F::from(section_id_prev as u64),
                    F::from(section_id as u64),
                )?;
                section_id_prev = section_id as i64;
            }

            self.assign(
                region,
                wasm_bytecode,
                section_start_offset,
                &[AssignType::IsSectionId],
                1,
                None,
            );

            let (_section_len, _section_len_leb_len) = self.markup_leb_section(
                region,
                &wasm_bytecode,
                section_len_start_offset,
                &[AssignType::IsSectionLen],
            );

            for i in 0..section_len {
                let offset = section_body_start_offset + i;
                self.assign(
                    region,
                    wasm_bytecode,
                    offset,
                    &[AssignType::IsSectionBody],
                    1,
                    None,
                );
            }

            if wasm_bytes_offset >= wasm_bytecode.bytes.len() { break }
        }

        let dynamic_indexes_offset = self.config.dynamic_indexes_chip.assign_auto(
            region,
            self.config.shared_state.borrow().dynamic_indexes_offset,
            self.config.shared_state.borrow().func_count,
            Tag::FuncIndex,
        ).unwrap();
        self.config.shared_state.borrow_mut().dynamic_indexes_offset = dynamic_indexes_offset;

        Ok(())
    }
}
