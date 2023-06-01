use halo2_proofs::{
    plonk::{Column, ConstraintSystem},
};
use std::{marker::PhantomData};
use halo2_proofs::circuit::{Chip, Layouter, Region, Value};
use halo2_proofs::plonk::{Advice, Constraints, Error, Fixed};
use halo2_proofs::poly::Rotation;
use eth_types::Field;
use gadgets::is_zero::{IsZeroChip, IsZeroConfig, IsZeroInstruction};
use gadgets::less_than::{LtChip, LtInstruction};
use gadgets::util::{and, Expr, not, or};
use crate::evm_circuit::util::constraint_builder::{BaseConstraintBuilder, ConstrainBuilderCommon};
use crate::table::PoseidonTable;
use crate::wasm_circuit::common::wasm_compute_section_len;
use crate::wasm_circuit::consts::{WASM_PREAMBLE_MAGIC_PREFIX, WASM_SECTIONS_START_INDEX, WASM_VERSION_PREFIX_BASE_INDEX, WASM_VERSION_PREFIX_LENGTH};
use crate::wasm_circuit::leb128_circuit::circuit::{LEB128Chip};
use crate::wasm_circuit::tables::range_table::RangeTableConfig;
use crate::wasm_circuit::wasm_bytecode::bytecode::WasmBytecode;
use crate::wasm_circuit::wasm_bytecode::bytecode_table::WasmBytecodeTable;
use crate::wasm_circuit::wasm_sections::consts::{ID_OF_SECTION_DEFAULT, WASM_SECTION_ID_MAX};

///
pub struct WasmSectionConfig<F: Field> {
    ///
    _marker: PhantomData<F>,
}

///
#[derive(Debug, Clone)]
pub struct WasmConfig<F: Field> {
    ///
    pub(crate) poseidon_table: PoseidonTable,
    ///
    pub(crate) byte_value_range_table_config: RangeTableConfig<F, 256>,
    ///
    pub(crate) section_id_range_table_config: RangeTableConfig<F, { WASM_SECTION_ID_MAX + 1 }>,
    ///
    pub(crate) wasm_bytecode_table: WasmBytecodeTable,
    ///
    section_len_leb_solid_number: Column<Advice>,
    ///
    q_enable: Column<Fixed>,
    ///
    q_first: Column<Fixed>,
    ///
    q_last: Column<Fixed>,
    ///
    index_at_positions: Vec<IsZeroConfig<F>>,
    ///
    index_at_prev_positions: Vec<IsZeroConfig<F>>,
    ///
    index_at_position_count: usize,
    ///
    is_section_id: Column<Advice>,
    ///
    is_section_len: Column<Advice>,
    ///
    id_of_section: Column<Advice>,
    ///
    is_section_body: Column<Advice>,
    /// TODO refactor to a single column
    /// array of LEB128Chip's where index+1 represents number of leb bytes
    leb128_chip_for_byte_count: Vec<LEB128Chip<F>>,
    ///
    is_id_of_section_grows_lt_chip: LtChip<F, 1>,
    ///
    _marker: PhantomData<F>,
}

impl<F: Field> WasmConfig<F>
{
    // pub fn get_leb_config(&self, leb_bytes_n: usize) -> &LEB128Config<F> {
    //     &self.leb128_config_for_byte_n[leb_bytes_n - 1]
    // }
    pub fn get_leb_chip(&self, leb_bytes_n: usize) -> &LEB128Chip<F> {
        &self.leb128_chip_for_byte_count[leb_bytes_n - 1]
    }
}


///
#[derive(Debug, Clone)]
pub struct WasmChip<F: Field> {
    ///
    pub config: WasmConfig<F>,
    _marker: PhantomData<F>,
}

impl<F: Field> WasmChip<F>
{
    ///
    pub fn load(
        &self,
        layouter: &mut impl Layouter<F>,
        wasm_bytecode: &WasmBytecode,
    ) -> Result<(), Error> {
        self.config.wasm_bytecode_table.load(layouter, wasm_bytecode)?;
        self.config.byte_value_range_table_config.load(layouter)?;
        self.config.section_id_range_table_config.load(layouter)?;

        self.config
            .poseidon_table
            .dev_load(layouter, &[wasm_bytecode.bytes.clone()])?;

        Ok(())
    }

    ///
    pub fn configure(
        cs: &mut ConstraintSystem<F>,
        wasm_bytecode_table: WasmBytecodeTable,
    ) -> WasmConfig<F> {
        let byte_value_range_table_config = RangeTableConfig::configure(cs);
        let section_id_range_table_config = RangeTableConfig::configure(cs);
        let index_at_position_count = WASM_PREAMBLE_MAGIC_PREFIX.len() + WASM_VERSION_PREFIX_LENGTH;
        let poseidon_table = PoseidonTable::dev_construct(cs);

        let q_enable = cs.fixed_column();
        let q_first = cs.fixed_column();
        let q_last = cs.fixed_column();
        let is_section_id = cs.advice_column();
        let id_of_section = cs.advice_column();
        let is_section_len = cs.advice_column();
        let is_section_body = cs.advice_column();
        let section_len_leb_solid_number = cs.advice_column();

        let mut leb_chips = Vec::new();
        for leb_bytes_n in 1..=10 {
            let config = LEB128Chip::configure(
                cs,
                |vc| vc.query_advice(section_len_leb_solid_number, Rotation::cur()),
                &wasm_bytecode_table.value,
                // TODO add support for signed
                false,
                leb_bytes_n,
            );
            let chip = LEB128Chip::construct(config);
            leb_chips.push(chip);
        }

        cs.lookup("all bytecode values are byte values", |vc| {
            let bytecode_value = vc.query_advice(wasm_bytecode_table.value, Rotation::cur());

            vec![(bytecode_value, byte_value_range_table_config.value)]
        });
        cs.create_gate("verify row", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            cb.require_boolean(
                "q_enable is boolean",
                vc.query_fixed(q_enable, Rotation::cur()),
            );
            cb.require_boolean(
                "q_first is boolean",
                vc.query_fixed(q_first, Rotation::cur()),
            );
            cb.require_boolean(
                "q_last is boolean",
                vc.query_fixed(q_last, Rotation::cur()),
            );
            cb.require_boolean(
                "is_section_id is boolean",
                vc.query_advice(is_section_id, Rotation::cur()),
            );
            cb.require_boolean(
                "is_section_len is boolean",
                vc.query_advice(is_section_len, Rotation::cur()),
            );
            cb.require_boolean(
                "is_section_body is boolean",
                vc.query_advice(is_section_body, Rotation::cur()),
            );
            cb.require_zero(
                "index == 0 when q_first == 1",
                and::expr([
                    vc.query_fixed(q_first, Rotation::cur()),
                    vc.query_advice(wasm_bytecode_table.index, Rotation::cur()),
                ]),
            );

            cb.gate(vc.query_fixed(q_enable, Rotation::cur()))
        });

        cs.create_gate("index grows by 1", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            cb.require_equal(
                "next.index == cur.index + 1",
                vc.query_advice(wasm_bytecode_table.index, Rotation::cur()) + 1.expr(),
                vc.query_advice(wasm_bytecode_table.index, Rotation::next()),
            );

            cb.gate(and::expr(vec![
                vc.query_fixed(q_enable, Rotation::cur()),
                not::expr(vc.query_fixed(q_last, Rotation::cur())),
            ]))
        });

        let mut index_at_positions: Vec<IsZeroConfig<F>> = Vec::new();
        for index in 0..index_at_position_count {
            let value_inv = cs.advice_column();
            let index_at_position = IsZeroChip::configure(
                cs,
                |vc| vc.query_fixed(q_enable, Rotation::cur()),
                |vc| vc.query_advice(wasm_bytecode_table.index, Rotation::cur()) - (index as i32).expr(),
                value_inv
            );
            index_at_positions.push(index_at_position);
        }
        let mut index_at_prev_positions: Vec<IsZeroConfig<F>> = Vec::new();
        for index in 0..index_at_position_count {
            let value_inv = cs.advice_column();
            let index_at_prev_position = IsZeroChip::configure(
                cs,
                |vc| and::expr([vc.query_fixed(q_enable, Rotation::cur()), not::expr(vc.query_fixed(q_first, Rotation::cur()))]),
                |vc| vc.query_advice(wasm_bytecode_table.index, Rotation::prev()) - (index as i32).expr(),
                value_inv
            );
            index_at_prev_positions.push(index_at_prev_position);
        }

        cs.create_gate("wasm gate: magic prefix check", |vc| {
            let mut cb = BaseConstraintBuilder::default();
            let bytecode_value = vc.query_advice(wasm_bytecode_table.value, Rotation::cur());

            // first bytes equal to '\0asm'
            for (i, char) in WASM_PREAMBLE_MAGIC_PREFIX.chars().enumerate() {
                cb.require_zero(
                    "bytecode.value == ord(char) at index",
                    and::expr([
                        index_at_positions[i].expr(),
                        bytecode_value.clone() - (char as i32).expr(),
                    ])
                );
            }
            for i in WASM_VERSION_PREFIX_BASE_INDEX..WASM_VERSION_PREFIX_BASE_INDEX + WASM_VERSION_PREFIX_LENGTH {
                let version_val = if i == WASM_VERSION_PREFIX_BASE_INDEX { 1 } else { 0 };
                cb.require_zero(
                    "bytecode.value == version_val at index",
                    and::expr([
                        index_at_positions[i].expr(),
                        vc.query_advice(wasm_bytecode_table.value, Rotation::cur()) - (version_val as i32).expr(),
                    ])
                );
            }

            cb.gate(and::expr(vec![
                vc.query_fixed(q_enable, Rotation::cur()),
            ]))
        });

        cs.create_gate("wasm gate: sections transitions check for magic prefix", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_enable_expr = vc.query_fixed(q_enable, Rotation::cur());
            let is_section_id_expr = vc.query_advice(is_section_id, Rotation::cur());
            let is_section_len_expr = vc.query_advice(is_section_len, Rotation::cur());
            let is_section_body_expr = vc.query_advice(is_section_body, Rotation::cur());

            for i in 0..WASM_SECTIONS_START_INDEX {
                cb.require_zero(
                    "bytecode[0]...bytecode[7] -> !is_section_id && !is_section_len && !is_section_body",
                    and::expr([
                        index_at_positions[i].expr(),
                        or::expr([is_section_id_expr.clone(), is_section_len_expr.clone(), is_section_body_expr.clone()]),
                    ]),
                );
            }

            cb.gate(and::expr(vec![
                q_enable_expr,
            ]))
        });
        cs.create_gate("wasm gate: section layout check", |vc| {
            let mut cb = BaseConstraintBuilder::default();

            let q_last_expr = vc.query_fixed(q_last, Rotation::cur());

            let bytecode_value = vc.query_advice(wasm_bytecode_table.value, Rotation::cur());

            let id_of_section_expr = vc.query_advice(id_of_section, Rotation::cur());

            let is_section_id_expr = vc.query_advice(is_section_id, Rotation::cur());
            let is_prev_section_id_expr = vc.query_advice(is_section_id, Rotation::prev());
            let is_section_len_expr = vc.query_advice(is_section_len, Rotation::cur());
            let is_prev_section_len_expr = vc.query_advice(is_section_len, Rotation::prev());
            let is_section_body_expr = vc.query_advice(is_section_body, Rotation::cur());
            let is_prev_section_body_expr = vc.query_advice(is_section_body, Rotation::prev());

            let section_len_leb_solid_number_expr = vc.query_advice(section_len_leb_solid_number, Rotation::cur());
            let section_len_leb_solid_number_prev_expr = vc.query_advice(section_len_leb_solid_number, Rotation::prev());

            cb.require_zero(
                "(only once, if previous bytecode index is 7) !is_section_id && !is_section_len && !is_section_body -(1)> is_section_id",
                and::expr([
                    index_at_prev_positions[WASM_SECTIONS_START_INDEX - 1].expr(),
                    or::expr([
                        is_section_id_expr.clone() - 1.expr(),
                        is_section_len_expr.clone(),
                        is_section_body_expr.clone(),
                    ])
                ]),
            );
            cb.condition(is_prev_section_id_expr.clone(), |bcb| {
                bcb.require_zero(
                    "is_section_id -(1)> is_section_len",
                    is_prev_section_id_expr.clone() - is_section_len_expr.clone(),
                );
            });
            cb.condition(is_prev_section_len_expr.clone(), |bcb| {
                bcb.require_zero(
                    "is_section_len -(N)> is_section_len || is_section_len -(1)> is_section_body",
                    is_prev_section_len_expr.clone() - is_section_len_expr.clone() - is_section_body_expr.clone(),
                );
            });
            cb.condition(is_prev_section_body_expr.clone(), |bcb| {
                bcb.require_zero(
                    "is_section_body -(N)> is_section_body || (shouldn't work for 'is_last') is_section_body -(N)> is_section_id",
                    is_prev_section_body_expr.clone() - is_section_body_expr.clone() - is_section_id_expr.clone(),
                );
            });
            cb.condition(q_last_expr.clone(), |bcb| {
                bcb.require_zero(
                    "is_section_body -(N)> is_section_body",
                    is_prev_section_body_expr.clone() - is_section_body_expr.clone(),
                );
            });

            cb.condition(
                is_section_id_expr.clone(),
                |bcb| {
                    bcb.require_equal(
                        "at 'is_section_id' - 'id_of_section' equal to 'bytecode.value'",
                        id_of_section_expr.clone(),
                        bytecode_value.clone(),
                    )
                }
            );

            cb.condition(
                is_section_len_expr.clone() * is_prev_section_len_expr.clone(),
                |bcb| {
                    bcb.require_zero(
                        "section_len_leb_solid_number must be equal for all section_len_leb_solid_number inside the same len block block",
                        section_len_leb_solid_number_expr.clone() - section_len_leb_solid_number_prev_expr.clone(),
                    );
                }
            );

            cb.condition(
                is_section_body_expr.clone(),
                |bcb| {
                    bcb.require_zero(
                        "section_len_leb_solid_number decreases by 1 for section_body",
                        section_len_leb_solid_number_prev_expr.clone() - section_len_leb_solid_number_expr.clone() - 1.expr(),
                    );
                }
            );
            cb.condition(
                or::expr([
                    is_section_id_expr.clone() * is_prev_section_body_expr.expr(),
                    q_last_expr.expr()
                ]),
                |bcb| {
                    bcb.require_zero(
                        "section_len_leb_solid_number_expr must equal 0 at the end of the body",
                        section_len_leb_solid_number_expr.clone(),
                    );
                }
            );

            cb.require_equal(
                "prev.hash == cur.hash",
                vc.query_advice(wasm_bytecode_table.code_hash, Rotation::prev()),
                vc.query_advice(wasm_bytecode_table.code_hash, Rotation::cur()),
            );

            cb.gate(and::expr(vec![
                not::expr(vc.query_fixed(q_first, Rotation::cur())),
                vc.query_fixed(q_enable, Rotation::cur()),
            ]))
        });

        cs.create_gate("at first 8 bytes 'id_of_section' equals to DEFAULT val", |vc| {
            let id_of_section_expr = vc.query_advice(id_of_section, Rotation::cur());

            let mut constraints = Vec::new();
            for i in 0..WASM_SECTIONS_START_INDEX {
                let constraint = index_at_positions[i].expr() * (id_of_section_expr.clone() - ID_OF_SECTION_DEFAULT.expr());
                constraints.push(
                    ("id of section equals to default at magic prefix indexes", constraint)
                );
            }
            Constraints::with_selector(
                vc.query_fixed(q_enable, Rotation::cur()),
                constraints,
            )
        });
        let is_id_of_section_grows_lt_chip_config = LtChip::configure(
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
                vc.query_advice(id_of_section, Rotation::prev())
            },
            |vc| {
                vc.query_advice(id_of_section, Rotation::cur())
            },
        );
        let is_id_of_section_grows_lt_chip = LtChip::construct(is_id_of_section_grows_lt_chip_config);
        cs.create_gate("prev.id_of_section <= cur.id_of_section", |vc| {
            let id_of_section_prev_expr = vc.query_advice(id_of_section, Rotation::prev());
            let id_of_section_expr = vc.query_advice(id_of_section, Rotation::cur());

            let mut constraints = Vec::new();

            constraints.push(
                ("prev.id_of_section <= cur.id_of_section",
                 (is_id_of_section_grows_lt_chip.config().is_lt(vc, None) - 1.expr())
                     * (id_of_section_expr.clone() - id_of_section_prev_expr.clone())
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
                    index_at_positions[2].expr(),
                    // vc.query_fixed(q_last, Rotation::cur()),
                    // not::expr(vc.query_fixed(q_first, Rotation::cur())),
                    vc.query_fixed(q_enable, Rotation::cur()),
                ]),
            )
        });

        cs.lookup("id_of_section is valid", |vc| {
            let id_of_section_expr = vc.query_advice(id_of_section, Rotation::cur());

            vec![(id_of_section_expr.clone(), section_id_range_table_config.value)]
        });

        let config = WasmConfig {
            poseidon_table,
            wasm_bytecode_table,
            section_len_leb_solid_number,
            q_enable,
            q_first,
            q_last,
            byte_value_range_table_config,
            section_id_range_table_config,
            index_at_positions,
            index_at_prev_positions,
            index_at_position_count,
            id_of_section,
            is_section_id,
            is_section_len,
            is_section_body,
            leb128_chip_for_byte_count: leb_chips,
            is_id_of_section_grows_lt_chip,
            _marker: PhantomData,
        };

        config
    }

    ///
    pub fn construct(config: WasmConfig<F>) -> Self {
        let instance = Self {
            config,
            _marker: PhantomData,
        };
        instance
    }

    ///
    pub fn assign(
        &self,
        region: &mut Region<F>,
        wasm_bytes: &[u8],
    ) -> Result<(), Error> {
        for chip in &self.config.leb128_chip_for_byte_count {
            chip.init_assign(region, wasm_bytes.len() - 1);
        }

        let mut index_at_positions: Vec<IsZeroChip<F>> = Vec::new();
        let mut index_at_prev_positions: Vec<IsZeroChip<F>> = Vec::new();
        for i in 0..self.config.index_at_position_count {
            index_at_positions.push(IsZeroChip::construct(self.config.index_at_positions[i].clone()));
        }
        for i in 0..self.config.index_at_position_count {
            index_at_prev_positions.push(IsZeroChip::construct(self.config.index_at_prev_positions[i].clone()));
        }

        for (i, &_byte) in wasm_bytes.iter().enumerate() {
            let is_enable = true;
            let is_first = if i == 0 { true } else { false };
            let is_last = if i == wasm_bytes.len() - 1 { true } else { false };
            region.assign_fixed(
                || format!("assign q_enable at {}", i),
                self.config.q_enable,
                i,
                || Value::known(F::from(is_enable as u64)),
            )?;
            region.assign_fixed(
                || format!("assign q_first at {}", i),
                self.config.q_first,
                i,
                || Value::known(F::from(is_first as u64)),
            )?;
            region.assign_fixed(
                || format!("assign q_last at {}", i),
                self.config.q_last,
                i,
                || Value::known(F::from(is_last as u64)),
            )?;

            region.assign_advice(
                || format!("assign is_section_id at {}", i),
                self.config.is_section_id,
                i,
                || Value::known(F::zero()),
            )?;
            region.assign_advice(
                || format!("assign is_section_len at {}", i),
                self.config.is_section_len,
                i,
                || Value::known(F::zero()),
            )?;
            region.assign_advice(
                || format!("assign is_section_body at {}", i),
                self.config.is_section_body,
                i,
                || Value::known(F::zero()),
            )?;

            let val: i64 = ID_OF_SECTION_DEFAULT as i64;
            region.assign_advice(
                || format!("assign id_of_section val {} at {}", val, i),
                self.config.id_of_section,
                i,
                || Value::known(if val < 0 { -F::from(val.abs() as u64) } else { F::from(val as u64) })
            )?;
            if i > 0 {
                self.config.is_id_of_section_grows_lt_chip.assign(
                    region,
                    i,
                    F::zero(),
                    F::zero(),
                )?;
            }

            for (index, index_at_position) in index_at_positions.iter().enumerate() {
                index_at_position.assign(region, i, Value::known(F::from(i as u64) - F::from(index as u64)))?;
            }
            for (index, index_at_prev_position) in index_at_prev_positions.iter().enumerate() {
                index_at_prev_position.assign(region, i, Value::known(F::from(i as u64) - F::from(index as u64) - F::from(1)))?;
            }
        }
        // scan wasm_bytes for sections
        let mut wasm_bytes_index = WASM_SECTIONS_START_INDEX;
        let mut section_id_prev: i64 = ID_OF_SECTION_DEFAULT as i64;
        loop {
            let section_start_index = wasm_bytes_index;
            let section_len_start_index = section_start_index + 1;
            let section_id = wasm_bytes[wasm_bytes_index];
            wasm_bytes_index += 1;
            let (section_len, section_len_leb_bytes_count) = wasm_compute_section_len(&wasm_bytes, wasm_bytes_index).unwrap();
            wasm_bytes_index += section_len_leb_bytes_count as usize;
            wasm_bytes_index += section_len;
            let section_body_start_index = section_len_start_index + (section_len_leb_bytes_count as usize);
            let section_len_end_index = section_body_start_index - 1;
            let section_body_end_index = section_start_index + section_len_leb_bytes_count as usize + section_len;
            let section_end_index = section_body_end_index;

            // println!();
            // println!("section_id_prev: {}", section_id_prev);
            // println!("section_id: {}", section_id);
            // println!("section_start_index: {}", section_start_index);
            // println!("section_start_index: {}", section_end_index);
            // println!("section_len: {}", section_len);
            // println!("section_len_start_index: {}", section_len_start_index);
            // println!("section_len_end_index: {}", section_len_end_index);
            // println!("section_body_start_index: {}", section_body_start_index);
            // println!("section_body_end_index: {}", section_body_end_index);
            // println!();

            {
                let offset = section_start_index;
                region.assign_advice(
                    || format!("assign is_section_id at {}", offset),
                    self.config.is_section_id,
                    offset,
                    || Value::known(F::one()),
                )?;
            }
            for i in 0..section_len_leb_bytes_count {
                let offset = section_len_start_index + (i as usize);
                region.assign_advice(
                    || format!("assign is_section_len at {}", offset),
                    self.config.is_section_len,
                    offset,
                    || Value::known(F::one()),
                )?;
            }
            for i in 0..section_len {
                let offset = section_body_start_index + (i as usize);
                region.assign_advice(
                    || format!("assign is_section_body at {}", offset),
                    self.config.is_section_body,
                    offset,
                    || Value::known(F::one()),
                )?;
            }

            let leb128_chip = self.config.get_leb_chip(section_len_leb_bytes_count as usize);
            let mut leb_base64_word: u64 = 0;
            for offset in section_len_start_index..=section_len_end_index {
                region.assign_advice(
                    || format!("assign section_len_leb_solid_number to {} at {}", section_len, offset),
                    self.config.section_len_leb_solid_number,
                    offset,
                    || Value::known(F::from(section_len as u64)),
                )?;
                leb_base64_word = leb_base64_word * 0b100000000 + wasm_bytes[section_len_start_index + (section_len_end_index-offset)] as u64;
            }
            for offset in section_len_start_index..=section_len_end_index {
                leb128_chip.assign(
                    region,
                    offset,
                    offset == section_len_start_index,
                    offset != section_len_end_index,
                    leb_base64_word as u64,
                );
            }
            let mut section_len_prev = section_len;
            for offset in section_body_start_index..=section_body_end_index {
                section_len_prev -= 1;
                region.assign_advice(
                    || format!("assign section_len_leb_solid_number to {} at {}", section_len_prev, offset),
                    self.config.section_len_leb_solid_number,
                    offset,
                    || Value::known(F::from(section_len_prev as u64)),
                )?;
            }

            for offset in section_start_index..=section_end_index {
                let val = section_id;
                region.assign_advice(
                    || format!("assign id_of_section val {} at {}", val, offset),
                    self.config.id_of_section,
                    offset,
                    || Value::known(F::from(section_id as u64))
                )?;
                self.config.is_id_of_section_grows_lt_chip.assign(
                    region,
                    offset,
                    F::from(section_id_prev as u64),
                    F::from(section_id as u64),
                )?;
                section_id_prev = section_id as i64;
            }

            if wasm_bytes_index >= wasm_bytes.len() { break }
        }

        Ok(())
    }
}