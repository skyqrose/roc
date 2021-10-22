use bumpalo::collections::Vec;
use bumpalo::Bump;
use core::panic;
use std::fmt::Debug;

use parity_wasm::elements::{Instruction, Instruction::*, Local, Serialize, VarUint32};
use roc_module::symbol::Symbol;

use crate::code_builder::VirtualMachineSymbolState;
use crate::opcodes::*;
use crate::{debug_panic, LocalId, STACK_POINTER_GLOBAL_ID};

const DEBUG_LOG: bool = false;

#[repr(u8)]
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum ValueType {
    I32 = 0x7f,
    I64 = 0x7e,
    F32 = 0x7d,
    F64 = 0x7c,
}

pub enum BlockType {
    NoResult,
    Value(ValueType),
}

impl BlockType {
    pub fn as_byte(&self) -> u8 {
        match self {
            Self::NoResult => 0x40,
            Self::Value(t) => *t as u8,
        }
    }
}

#[repr(u8)]
pub enum Align {
    Bytes1 = 0,
    Bytes2 = 1,
    Bytes4 = 2,
    Bytes8 = 3,
    Bytes16 = 4,
    Bytes32 = 5,
    Bytes64 = 6,
    // ... we can add more if we need them ...
}

macro_rules! instruction_no_args {
    ($method_name: ident, $pops: expr, $push: expr, $opcode: expr) => {
        pub fn $method_name(&mut self) {
            self.inst($pops, $push, $opcode);
        }
    };
}

macro_rules! instruction_memargs {
    ($method_name: ident, $pops: expr, $push: expr, $opcode: expr) => {
        pub fn $method_name(&mut self, align: Align, offset: u32) {
            self.inst_mem($pops, $push, $opcode, align, offset);
        }
    };
}

#[derive(Debug)]
pub struct FunctionBuilder<'a> {
    /// The main container for the instructions
    code: Vec<'a, u8>,

    /// Extra instructions to insert at specific positions during finalisation
    /// (Go back and set locals when we realise we need them)
    insertions: Vec<'a, (usize, Vec<'a, u8>)>,

    /// Instruction bytes for locals and stack frame setup code
    locals_and_frame_setup: Vec<'a, u8>,

    /// Encoded bytes for the inner length of the function
    /// This is the total size of locals + code, as encoded into the module
    inner_length: Vec<'a, u8>,

    /// Our simulation model of the Wasm stack machine
    /// Keeps track of where Symbol values are in the VM stack
    vm_stack: Vec<'a, Symbol>,
}

#[allow(clippy::new_without_default)]
impl<'a> FunctionBuilder<'a> {
    pub fn new(arena: &'a Bump) -> Self {
        FunctionBuilder {
            code: Vec::with_capacity_in(1024, arena),
            insertions: Vec::with_capacity_in(1024, arena),
            locals_and_frame_setup: Vec::with_capacity_in(32, arena),
            inner_length: Vec::with_capacity_in(5, arena),
            vm_stack: Vec::with_capacity_in(32, arena),
        }
    }

    pub fn clear(&mut self) {
        self.code.clear();
        self.insertions.clear();
        self.vm_stack.clear();
    }

    /// Set the Symbol that is at the top of the VM stack right now
    /// We will use this later when we need to load the Symbol
    pub fn set_top_symbol(&mut self, sym: Symbol) -> VirtualMachineSymbolState {
        let len = self.vm_stack.len();
        let pushed_at = self.code.len();

        if len == 0 {
            panic!(
                "trying to set symbol with nothing on stack, code = {:?}",
                self.code
            );
        }

        self.vm_stack[len - 1] = sym;

        VirtualMachineSymbolState::Pushed { pushed_at }
    }

    /// Verify if a sequence of symbols is at the top of the stack
    pub fn verify_stack_match(&self, symbols: &[Symbol]) -> bool {
        let n_symbols = symbols.len();
        let stack_depth = self.vm_stack.len();
        if n_symbols > stack_depth {
            return false;
        }
        let offset = stack_depth - n_symbols;

        for (i, sym) in symbols.iter().enumerate() {
            if self.vm_stack[offset + i] != *sym {
                return false;
            }
        }
        true
    }

    fn push_stack_frame(frame_size: i32, local_frame_pointer: LocalId) -> [Instruction; 5] {
        return [
            GetGlobal(STACK_POINTER_GLOBAL_ID),
            I32Const(frame_size),
            I32Sub,
            TeeLocal(local_frame_pointer.0),
            SetGlobal(STACK_POINTER_GLOBAL_ID),
        ];
    }

    fn pop_stack_frame(frame_size: i32, local_frame_pointer: LocalId) -> [Instruction; 4] {
        return [
            GetLocal(local_frame_pointer.0),
            I32Const(frame_size),
            I32Add,
            SetGlobal(STACK_POINTER_GLOBAL_ID),
        ];
    }

    fn serialize_locals<W>(writer: &mut W, local_types: &[parity_wasm::elements::ValueType])
    where
        W: std::io::Write,
    {
        let num_locals = local_types.len();
        VarUint32::from(num_locals)
            .serialize(writer)
            .unwrap_or_else(debug_panic);

        // write bytes for locals, in batches of the same ValueType
        if num_locals > 0 {
            let mut batch_type = local_types[0];
            let mut batch_size = 0;
            for t in local_types {
                if *t == batch_type {
                    batch_size += 1;
                } else {
                    let local = Local::new(batch_size, batch_type);
                    local.serialize(writer).unwrap_or_else(debug_panic);
                    batch_type = *t;
                    batch_size = 1;
                }
            }
            let local = Local::new(batch_size, batch_type);
            local.serialize(writer).unwrap_or_else(debug_panic);
        }
    }

    /// Finalize a function body
    pub fn finalize(&mut self, _final_code: &mut std::vec::Vec<Instruction>) {
        // sort insertions
        // encode local declarations
        // encode stack frame push
        // encode stack frame pop
        // calculate inner length
        // encode inner length
    }

    pub fn serialize<W: std::io::Write>(&self, _writer: W) {
        // write inner length
        // write locals
        // write stack frame push
        // write code+insertions
        // write stack frame pop
    }

    /// Total bytes, including inner length
    /// (to help calculate overall code section length)
    pub fn outer_len(&self) -> usize {
        self.code.len()
            + self.insertions.len()
            + self.locals_and_frame_setup.len()
            + self.inner_length.len()
    }

    pub fn call(&mut self, function_index: u32, pops: usize, push: bool) {
        let stack_depth = self.vm_stack.len();
        if pops > stack_depth {
            panic!(
                "Trying to call to call function {:?} with {:?} values but only {:?} on the VM stack\n{:?}",
                function_index, pops, stack_depth, self
            );
        }
        self.vm_stack.truncate(stack_depth - pops);
        if push {
            self.vm_stack.push(Symbol::WASM_ANONYMOUS_STACK_VALUE);
        }
        self.code.push(CALL);
    }

    /// Base method for generating instructions
    /// Emits the opcode and simulates VM stack push/pop
    fn inst(&mut self, pops: usize, push: bool, opcode: u8) {
        let new_len = self.vm_stack.len() - pops as usize;
        self.vm_stack.truncate(new_len);
        if push {
            self.vm_stack.push(Symbol::WASM_ANONYMOUS_STACK_VALUE);
        }
        self.code.push(opcode);
    }

    fn inst_imm8(&mut self, pops: usize, push: bool, opcode: u8, immediate: u8) {
        self.inst(pops, push, opcode);
        self.code.push(immediate);
    }

    fn inst_imm32(&mut self, pops: usize, push: bool, opcode: u8, immediate: u32) {
        self.inst(pops, push, opcode);
        let mut value = immediate;
        while value >= 0x80 {
            self.code.push(0x80 | ((value & 0x7f) as u8));
            value >>= 7;
        }
        self.code.push(value as u8);
    }

    fn inst_mem(&mut self, pops: usize, push: bool, opcode: u8, align: Align, offset: u32) {
        self.inst(pops, push, opcode);
        self.code.push(align as u8);
        let mut value = offset;
        while value >= 0x80 {
            self.code.push(0x80 | ((value & 0x7f) as u8));
            value >>= 7;
        }
        self.code.push(value as u8);
    }

    // Instruction methods
    //
    // One method for each Wasm instruction (in same order as the spec)
    // macros are just for compactness and readability for the most common cases
    // Patterns that don't repeat very much don't have macros
    // instruction_no_args! creates a method that takes no arguments
    // instruction_memargs! creates a method that takes alignment and offset arguments

    instruction_no_args!(unreachable_, 0, false, UNREACHABLE);
    instruction_no_args!(nop, 0, false, NOP);
    pub fn block(&mut self, ty: BlockType) {
        self.inst_imm8(0, false, BLOCK, ty.as_byte());
    }
    pub fn loop_(&mut self, ty: BlockType) {
        self.inst_imm8(0, false, LOOP, ty.as_byte());
    }
    pub fn if_(&mut self, ty: BlockType) {
        self.inst_imm8(1, false, IF, ty.as_byte());
    }
    instruction_no_args!(else_, 0, false, ELSE);
    instruction_no_args!(end, 0, false, END);
    pub fn br(&mut self, levels: u32) {
        self.inst_imm32(0, false, BR, levels);
    }
    pub fn br_if(&mut self, levels: u32) {
        self.inst_imm32(1, false, BRIF, levels);
    }
    // br_table: not implemented
    instruction_no_args!(return_, 0, false, RETURN);
    // call: see above
    // call_indirect: not implemented
    instruction_no_args!(drop, 1, false, DROP);
    instruction_no_args!(select, 3, true, SELECT);
    pub fn get_local(&mut self, id: LocalId) {
        self.inst_imm32(0, true, GETLOCAL, id.0);
    }
    pub fn set_local(&mut self, id: LocalId) {
        self.inst_imm32(1, false, SETLOCAL, id.0);
    }
    pub fn tee_local(&mut self, id: LocalId) {
        self.inst_imm32(1, true, TEELOCAL, id.0);
    }
    pub fn get_global(&mut self, id: u32) {
        self.inst_imm32(0, true, GETGLOBAL, id);
    }
    pub fn set_global(&mut self, id: u32) {
        self.inst_imm32(1, false, SETGLOBAL, id);
    }
    instruction_memargs!(i32_load, 1, true, I32LOAD);
    instruction_memargs!(i64_load, 1, true, I64LOAD);
    instruction_memargs!(f32_load, 1, true, F32LOAD);
    instruction_memargs!(f64_load, 1, true, F64LOAD);
    instruction_memargs!(i32_load8_s, 1, true, I32LOAD8S);
    instruction_memargs!(i32_load8_u, 1, true, I32LOAD8U);
    instruction_memargs!(i32_load16_s, 1, true, I32LOAD16S);
    instruction_memargs!(i32_load16_u, 1, true, I32LOAD16U);
    instruction_memargs!(i64_load8_s, 1, true, I64LOAD8S);
    instruction_memargs!(i64_load8_u, 1, true, I64LOAD8U);
    instruction_memargs!(i64_load16_s, 1, true, I64LOAD16S);
    instruction_memargs!(i64_load16_u, 1, true, I64LOAD16U);
    instruction_memargs!(i64_load32_s, 1, true, I64LOAD32S);
    instruction_memargs!(i64_load32_u, 1, true, I64LOAD32U);
    instruction_memargs!(i32_store, 2, false, I32STORE);
    instruction_memargs!(i64_store, 2, false, I64STORE);
    instruction_memargs!(f32_store, 2, false, F32STORE);
    instruction_memargs!(f64_store, 2, false, F64STORE);
    instruction_memargs!(i32_store8, 2, false, I32STORE8);
    instruction_memargs!(i32_store16, 2, false, I32STORE16);
    instruction_memargs!(i64_store8, 2, false, I64STORE8);
    instruction_memargs!(i64_store16, 2, false, I64STORE16);
    instruction_memargs!(i64_store32, 2, false, I64STORE32);
    pub fn memory_size(&mut self) {
        self.inst_imm8(0, true, CURRENTMEMORY, 0);
    }
    pub fn memory_grow(&mut self) {
        self.inst_imm8(1, true, GROWMEMORY, 0);
    }
    pub fn i32_const(&mut self, x: i32) {
        self.inst_imm32(0, true, I32CONST, x as u32);
    }
    pub fn i64_const(&mut self, x: i64) {
        self.inst(0, true, I64CONST);
        let mut value = x as u64;
        while value >= 0x80 {
            self.code.push(0x80 | ((value & 0x7f) as u8));
            value >>= 7;
        }
        self.code.push(value as u8);
    }
    pub fn f32_const(&mut self, x: f32) {
        self.inst(0, true, F32CONST);
        // No LEB encoding, and always little-endian regardless of compiler host.
        let mut value: u32 = x.to_bits();
        for _ in 0..4 {
            self.code.push((value & 0xff) as u8);
            value >>= 8;
        }
    }
    pub fn f64_const(&mut self, x: f64) {
        self.inst(0, true, F64CONST);
        // No LEB encoding, and always little-endian regardless of compiler host.
        let mut value: u64 = x.to_bits();
        for _ in 0..8 {
            self.code.push((value & 0xff) as u8);
            value >>= 8;
        }
    }
    instruction_no_args!(i32_eqz, 1, true, I32EQZ);
    instruction_no_args!(i32_eq, 2, true, I32EQ);
    instruction_no_args!(i32_ne, 2, true, I32NE);
    instruction_no_args!(i32_lt_s, 2, true, I32LTS);
    instruction_no_args!(i32_lt_u, 2, true, I32LTU);
    instruction_no_args!(i32_gt_s, 2, true, I32GTS);
    instruction_no_args!(i32_gt_u, 2, true, I32GTU);
    instruction_no_args!(i32_le_s, 2, true, I32LES);
    instruction_no_args!(i32_le_u, 2, true, I32LEU);
    instruction_no_args!(i32_ge_s, 2, true, I32GES);
    instruction_no_args!(i32_ge_u, 2, true, I32GEU);
    instruction_no_args!(i64_eqz, 1, true, I64EQZ);
    instruction_no_args!(i64_eq, 2, true, I64EQ);
    instruction_no_args!(i64_ne, 2, true, I64NE);
    instruction_no_args!(i64_lt_s, 2, true, I64LTS);
    instruction_no_args!(i64_lt_u, 2, true, I64LTU);
    instruction_no_args!(i64_gt_s, 2, true, I64GTS);
    instruction_no_args!(i64_gt_u, 2, true, I64GTU);
    instruction_no_args!(i64_le_s, 2, true, I64LES);
    instruction_no_args!(i64_le_u, 2, true, I64LEU);
    instruction_no_args!(i64_ge_s, 2, true, I64GES);
    instruction_no_args!(i64_ge_u, 2, true, I64GEU);
    instruction_no_args!(f32_eq, 2, true, F32EQ);
    instruction_no_args!(f32_ne, 2, true, F32NE);
    instruction_no_args!(f32_lt, 2, true, F32LT);
    instruction_no_args!(f32_gt, 2, true, F32GT);
    instruction_no_args!(f32_le, 2, true, F32LE);
    instruction_no_args!(f32_ge, 2, true, F32GE);
    instruction_no_args!(f64_eq, 2, true, F64EQ);
    instruction_no_args!(f64_ne, 2, true, F64NE);
    instruction_no_args!(f64_lt, 2, true, F64LT);
    instruction_no_args!(f64_gt, 2, true, F64GT);
    instruction_no_args!(f64_le, 2, true, F64LE);
    instruction_no_args!(f64_ge, 2, true, F64GE);
    instruction_no_args!(i32_clz, 1, true, I32CLZ);
    instruction_no_args!(i32_ctz, 1, true, I32CTZ);
    instruction_no_args!(i32_popcnt, 1, true, I32POPCNT);
    instruction_no_args!(i32_add, 2, true, I32ADD);
    instruction_no_args!(i32_sub, 2, true, I32SUB);
    instruction_no_args!(i32_mul, 2, true, I32MUL);
    instruction_no_args!(i32_div_s, 2, true, I32DIVS);
    instruction_no_args!(i32_div_u, 2, true, I32DIVU);
    instruction_no_args!(i32_rem_s, 2, true, I32REMS);
    instruction_no_args!(i32_rem_u, 2, true, I32REMU);
    instruction_no_args!(i32_and, 2, true, I32AND);
    instruction_no_args!(i32_or, 2, true, I32OR);
    instruction_no_args!(i32_xor, 2, true, I32XOR);
    instruction_no_args!(i32_shl, 2, true, I32SHL);
    instruction_no_args!(i32_shr_s, 2, true, I32SHRS);
    instruction_no_args!(i32_shr_u, 2, true, I32SHRU);
    instruction_no_args!(i32_rotl, 2, true, I32ROTL);
    instruction_no_args!(i32_rotr, 2, true, I32ROTR);
    instruction_no_args!(i64_clz, 1, true, I64CLZ);
    instruction_no_args!(i64_ctz, 1, true, I64CTZ);
    instruction_no_args!(i64_popcnt, 1, true, I64POPCNT);
    instruction_no_args!(i64_add, 2, true, I64ADD);
    instruction_no_args!(i64_sub, 2, true, I64SUB);
    instruction_no_args!(i64_mul, 2, true, I64MUL);
    instruction_no_args!(i64_div_s, 2, true, I64DIVS);
    instruction_no_args!(i64_div_u, 2, true, I64DIVU);
    instruction_no_args!(i64_rem_s, 2, true, I64REMS);
    instruction_no_args!(i64_rem_u, 2, true, I64REMU);
    instruction_no_args!(i64_and, 2, true, I64AND);
    instruction_no_args!(i64_or, 2, true, I64OR);
    instruction_no_args!(i64_xor, 2, true, I64XOR);
    instruction_no_args!(i64_shl, 2, true, I64SHL);
    instruction_no_args!(i64_shr_s, 2, true, I64SHRS);
    instruction_no_args!(i64_shr_u, 2, true, I64SHRU);
    instruction_no_args!(i64_rotl, 2, true, I64ROTL);
    instruction_no_args!(i64_rotr, 2, true, I64ROTR);
    instruction_no_args!(f32_abs, 1, true, F32ABS);
    instruction_no_args!(f32_neg, 1, true, F32NEG);
    instruction_no_args!(f32_ceil, 1, true, F32CEIL);
    instruction_no_args!(f32_floor, 1, true, F32FLOOR);
    instruction_no_args!(f32_trunc, 1, true, F32TRUNC);
    instruction_no_args!(f32_nearest, 1, true, F32NEAREST);
    instruction_no_args!(f32_sqrt, 1, true, F32SQRT);
    instruction_no_args!(f32_add, 2, true, F32ADD);
    instruction_no_args!(f32_sub, 2, true, F32SUB);
    instruction_no_args!(f32_mul, 2, true, F32MUL);
    instruction_no_args!(f32_div, 2, true, F32DIV);
    instruction_no_args!(f32_min, 2, true, F32MIN);
    instruction_no_args!(f32_max, 2, true, F32MAX);
    instruction_no_args!(f32_copysign, 2, true, F32COPYSIGN);
    instruction_no_args!(f64_abs, 1, true, F64ABS);
    instruction_no_args!(f64_neg, 1, true, F64NEG);
    instruction_no_args!(f64_ceil, 1, true, F64CEIL);
    instruction_no_args!(f64_floor, 1, true, F64FLOOR);
    instruction_no_args!(f64_trunc, 1, true, F64TRUNC);
    instruction_no_args!(f64_nearest, 1, true, F64NEAREST);
    instruction_no_args!(f64_sqrt, 1, true, F64SQRT);
    instruction_no_args!(f64_add, 2, true, F64ADD);
    instruction_no_args!(f64_sub, 2, true, F64SUB);
    instruction_no_args!(f64_mul, 2, true, F64MUL);
    instruction_no_args!(f64_div, 2, true, F64DIV);
    instruction_no_args!(f64_min, 2, true, F64MIN);
    instruction_no_args!(f64_max, 2, true, F64MAX);
    instruction_no_args!(f64_copysign, 2, true, F64COPYSIGN);
    instruction_no_args!(i32_wrap_i64, 1, true, I32WRAPI64);
    instruction_no_args!(i32_trunc_s_f32, 1, true, I32TRUNCSF32);
    instruction_no_args!(i32_trunc_u_f32, 1, true, I32TRUNCUF32);
    instruction_no_args!(i32_trunc_s_f64, 1, true, I32TRUNCSF64);
    instruction_no_args!(i32_trunc_u_f64, 1, true, I32TRUNCUF64);
    instruction_no_args!(i64_extend_s_i32, 1, true, I64EXTENDSI32);
    instruction_no_args!(i64_extend_u_i32, 1, true, I64EXTENDUI32);
    instruction_no_args!(i64_trunc_s_f32, 1, true, I64TRUNCSF32);
    instruction_no_args!(i64_trunc_u_f32, 1, true, I64TRUNCUF32);
    instruction_no_args!(i64_trunc_s_f64, 1, true, I64TRUNCSF64);
    instruction_no_args!(i64_trunc_u_f64, 1, true, I64TRUNCUF64);
    instruction_no_args!(f32_convert_s_i32, 1, true, F32CONVERTSI32);
    instruction_no_args!(f32_convert_u_i32, 1, true, F32CONVERTUI32);
    instruction_no_args!(f32_convert_s_i64, 1, true, F32CONVERTSI64);
    instruction_no_args!(f32_convert_u_i64, 1, true, F32CONVERTUI64);
    instruction_no_args!(f32_demote_f64, 1, true, F32DEMOTEF64);
    instruction_no_args!(f64_convert_s_i32, 1, true, F64CONVERTSI32);
    instruction_no_args!(f64_convert_u_i32, 1, true, F64CONVERTUI32);
    instruction_no_args!(f64_convert_s_i64, 1, true, F64CONVERTSI64);
    instruction_no_args!(f64_convert_u_i64, 1, true, F64CONVERTUI64);
    instruction_no_args!(f64_promote_f32, 1, true, F64PROMOTEF32);
    instruction_no_args!(i32_reinterpret_f32, 1, true, I32REINTERPRETF32);
    instruction_no_args!(i64_reinterpret_f64, 1, true, I64REINTERPRETF64);
    instruction_no_args!(f32_reinterpret_i32, 1, true, F32REINTERPRETI32);
    instruction_no_args!(f64_reinterpret_i64, 1, true, F64REINTERPRETI64);
}
