mod symbols;

use std::borrow::Borrow;
use symbols::{WastSymbol, WastSymbolTable};

use jswt_ast::high_level::*;
use jswt_common::{PrimitiveType, SemanticSymbolTable, Type};
use jswt_wast::*;

#[derive(Debug)]
struct InstructionScope {
    instructions: Vec<Instruction>,
}

#[derive(Debug)]
pub struct CodeGenerator {
    module: Module,
    /// The architecture here assumes that this is an instruction scope stack
    /// any lexical scope that wishes to recieve an emitted set of instructions
    /// should push a new scope to the stack and pop the scope before pushing their own
    /// instruction to the stack
    scopes: Vec<InstructionScope>,
    semantic_symbols: SemanticSymbolTable,
    wast_symbols: WastSymbolTable,
    label_counter: usize,
}

impl Default for CodeGenerator {
    fn default() -> Self {
        Self {
            wast_symbols: WastSymbolTable::new(),
            module: Default::default(),
            scopes: Default::default(),
            semantic_symbols: Default::default(),
            label_counter: Default::default(),
        }
    }
}

impl CodeGenerator {
    pub fn new(semantic_symbols: SemanticSymbolTable) -> Self {
        Self {
            semantic_symbols,
            ..Default::default()
        }
    }

    pub fn generate_module(&mut self, ast: &Ast) -> &Module {
        // TODO - we should be accepting builtins externally from the env
        // This is a stop gap so tests don't break
        self.visit_program(&ast.program);
        &self.module
    }

    fn push_import(&mut self, import: Import) -> usize {
        self.module.imports.push(import);
        self.module.imports.len() - 1
    }

    fn push_global(&mut self, global: GlobalType) -> usize {
        self.module.globals.push(global);
        self.module.globals.len() - 1
    }

    fn push_type(&mut self, new_ty: FunctionType) -> usize {
        for (i, ty) in self.module.types.iter().enumerate() {
            if *ty == new_ty {
                return i;
            }
        }
        self.module.types.push(new_ty);
        self.module.types.len() - 1
    }

    fn push_function(&mut self, func: Function) -> usize {
        self.module.functions.push(func);
        self.module.functions.len() - 1
    }

    fn push_export(&mut self, export: Export) -> usize {
        self.module.exports.push(export);
        self.module.functions.len() - 1
    }

    fn push_instruction(&mut self, inst: Instruction) {
        let scope = self.scopes.last_mut().unwrap();
        scope.instructions.push(inst);
    }

    /// Adds a new instruction scope context to the stack
    fn push_instruction_scope(&mut self) {
        self.scopes.push(InstructionScope {
            instructions: vec![],
        });
    }

    /// Pops an Instruction scope from the stack
    fn pop_instruction_scope(&mut self) -> Option<InstructionScope> {
        self.scopes.pop()
    }
}

impl StatementVisitor for CodeGenerator {
    fn visit_program(&mut self, node: &Program) {
        // Push global scope
        self.semantic_symbols.push_global_scope();
        self.wast_symbols.push_scope();

        self.visit_source_elements(&node.source_elements);

        // Pop global scope from stack
        debug_assert_eq!(self.semantic_symbols.depth(), 1);
        self.semantic_symbols.pop_scope();
        self.wast_symbols.pop_scope();
    }

    fn visit_source_elements(&mut self, node: &SourceElements) {
        for element in &node.source_elements {
            self.visit_source_element(element);
        }
    }

    fn visit_source_element(&mut self, node: &SourceElement) {
        match node {
            SourceElement::FunctionDeclaration(function) => {
                self.visit_function_declaration(function)
            }
            SourceElement::Statement(statement) => self.visit_statement_element(statement),
        }
    }

    fn visit_statement_element(&mut self, node: &StatementElement) {
        match node {
            StatementElement::Block(stmt) => self.visit_block_statement(stmt),
            StatementElement::Empty(stmt) => self.visit_empty_statement(stmt),
            StatementElement::Return(stmt) => self.visit_return_statement(stmt),
            StatementElement::Variable(stmt) => self.visit_variable_statement(stmt),
            StatementElement::Expression(stmt) => self.visit_expression_statement(stmt),
            StatementElement::If(stmt) => self.visit_if_statement(stmt),
            StatementElement::Iteration(stmt) => self.visit_iteration_statement(stmt),
        }
    }

    fn visit_block_statement(&mut self, node: &BlockStatement) {
        self.visit_statement_list(&node.statements);
    }

    fn visit_empty_statement(&mut self, _: &EmptyStatement) {
        // No-op
    }

    fn visit_if_statement(&mut self, node: &IfStatement) {
        let cond = self.visit_single_expression(&node.condition);

        // Handle the consequence instructions
        self.push_instruction_scope();
        self.visit_statement_element(&node.consequence);
        let cons = self.pop_instruction_scope().unwrap();

        // Handle the alternative instructions
        self.push_instruction_scope();
        if let Some(alternative) = node.alternative.borrow() {
            self.visit_statement_element(alternative);
        }
        let alt = self.pop_instruction_scope().unwrap();

        self.push_instruction(Instruction::If(
            Box::new(cond),
            cons.instructions,
            alt.instructions,
        ));
    }

    fn visit_iteration_statement(&mut self, node: &IterationStatement) {
        match node {
            IterationStatement::While(elem) => self.visit_while_iteration_element(elem),
        }
    }

    fn visit_while_iteration_element(&mut self, node: &WhileIterationElement) {
        let loop_label = self.label_counter;
        self.label_counter += 1;

        self.push_instruction_scope();

        // First push the expression result onto the stack
        let cond = self.visit_single_expression(&node.expression);

        // Push an if scope for branching
        self.push_instruction_scope();
        // Add the statements to the scope
        self.visit_statement_element(&node.statement);

        // Add the branch back to the top of the loop to test
        self.push_instruction(Instruction::BrLoop(loop_label));

        let if_scope = self.pop_instruction_scope().unwrap();
        self.push_instruction(Instruction::If(
            Box::new(cond),
            if_scope.instructions,
            vec![],
        ));

        let loop_scope = self.pop_instruction_scope().unwrap();
        self.push_instruction(Instruction::Loop(loop_label, loop_scope.instructions));
    }

    fn visit_return_statement(&mut self, node: &ReturnStatement) {
        let exp = self.visit_single_expression(&node.expression);
        self.push_instruction(Instruction::Return(Box::new(exp)));
    }

    fn visit_variable_statement(&mut self, node: &VariableStatement) {
        // This should be assignment local.set, global.set
        let target = self.visit_assignable_element(&node.target);
        let exp = self.visit_single_expression(&node.expression);
        match target {
            Instruction::GlobalSet(name, _) => {
                self.push_global(GlobalType {
                    name,
                    ty: ValueType::I32,
                    mutable: true, // TODO - check mutability
                    initializer: exp,
                });
            }
            Instruction::LocalSet(name, _) => {
                let exp = Instruction::LocalSet(name, Box::new(exp));
                self.push_instruction(exp)
            }
            _ => self.push_instruction(exp),
        }
    }

    fn visit_expression_statement(&mut self, node: &ExpressionStatement) {
        let isr = self.visit_single_expression(&node.expression);
        self.push_instruction(isr);
    }

    fn visit_statement_list(&mut self, node: &StatementList) {
        for statement in &node.statements {
            self.visit_statement_element(statement);
        }
    }

    fn visit_function_declaration(&mut self, node: &FunctionDeclarationElement) {
        let function_name = node.ident.value;

        // Push the scope for the function body
        // Should already exist on the symbol table
        self.semantic_symbols.push_scope(node.span(), None);
        self.wast_symbols.push_scope();

        let mut type_params = vec![];
        // Push Symbols for Params. We need this in case the scope
        // needs to declare synthetic local variables
        for (index, arg) in node.params.parameters.iter().enumerate() {
            // Add to symbol table
            let sym = self
                .semantic_symbols
                .lookup(arg.ident.value.into())
                .unwrap();
            let ty = ValueType::from(sym.ty.clone());

            type_params.push((arg.ident.value, ty));
            // Add to wast symbol table
            self.wast_symbols
                .define(arg.ident.value.into(), WastSymbol::Param(index, ty));
        }

        // Resolve return Value
        let scope = self.semantic_symbols.get_scope(node.span()).unwrap();
        let return_type = scope.returns.clone().map(ValueType::from);

        let ty = FunctionType {
            params: type_params,
            ret: return_type.clone(),
        };
        // Add type definition to type index
        let type_idx = self.push_type(ty);

        // Push a new Instruction scope to hold emitted instructions
        self.push_instruction_scope();

        // Resolve annotations
        let mut has_inlined_body = false;
        let mut is_predefined_function = false;
        for annotation in &node.decorators.annotations {
            match annotation.name.value {
                // The "wast" annotation allows the developer to emit
                // WAST instructions directily into the instruction scope
                // of the function.
                "wast" => match &annotation.expr {
                    Some(SingleExpression::Literal(Literal::String(string_lit))) => {
                        has_inlined_body = true;
                        self.push_instruction(Instruction::RawWast(string_lit.value.into()));
                    }
                    _ => todo!(),
                },
                "native" => match &annotation.expr {
                    Some(SingleExpression::Literal(Literal::String(string_lit))) => {
                        has_inlined_body = true;
                        is_predefined_function = true;
                        self.push_import(Import::Function(FunctionImport {
                            name: function_name,
                            type_idx,
                            module: string_lit.value,
                        }));
                    }
                    _ => todo!(),
                },
                "inline" => {}
                _ => {}
            }
        }

        let mut instructions = vec![];
        // Generate instructions for the current scope context
        // If we haven't already inlined a function body
        if !has_inlined_body {
            self.visit_function_body(&node.body);
            // Function generation is done. Pop the current instructions scope
            // and commit it to the module
            let scope = self.pop_instruction_scope().unwrap();

            instructions.push(Instruction::Block(0, scope.instructions));
            // Add synthetic return value
            // This is to make dealing with branching returns easier to manage
            // We're using a keyword here to prevent users from accidentally shadowing the value
            if let Some(return_type) = return_type {
                self.wast_symbols
                    .define("return".into(), WastSymbol::Local(return_type));

                // We're pushing the synthetic return to the end of the function
                instructions.push(Instruction::SynthReturn);
            }
        } else {
            // Just use the scope as is we assume that the user knows what
            // they are doing.
            instructions = self.pop_instruction_scope().unwrap().instructions;
        }

        // Push our locals to the instructions
        for (name, sym) in self.wast_symbols.symbols_in_current_scope() {
            if let WastSymbol::Local(ty) = sym {
                instructions.insert(0, Instruction::Local(name.clone(), *ty))
            }
        }

        if !is_predefined_function {
            let function = Function {
                name: function_name,
                type_idx,
                instructions,
            };
            let function_idx = self.push_function(function);

            // Generate export descriptor if the function is marked for export
            if node.decorators.export {
                let desc = FunctionExport {
                    function_idx,
                    name: function_name,
                };
                self.push_export(Export::Function(desc));
            }
        }

        // Pop the current function scope from the symbol table
        self.wast_symbols.pop_scope();
        self.semantic_symbols.pop_scope();
    }

    fn visit_function_body(&mut self, node: &FunctionBody) {
        self.visit_source_elements(&node.source_elements);
    }
}
impl ExpressionVisitor<Instruction> for CodeGenerator {
    fn visit_assignment_expression(&mut self, node: &BinaryExpression) -> Instruction {
        let rhs = self.visit_single_expression(node.right.borrow());

        match node.left.borrow() {
            SingleExpression::Identifier(ident_exp) => {
                let name = ident_exp.ident.value;
                // figure out the scope of the variable
                let isr = if self.wast_symbols.lookup_global(name.into()).is_some() {
                    Instruction::GlobalSet
                } else {
                    Instruction::LocalSet
                };
                isr(name.into(), Box::new(rhs))
            }
            SingleExpression::MemberIndex(exp) => {
                let index_ptr = self.visit_member_index(exp);
                Instruction::I32Store(Box::new(index_ptr), Box::new(rhs))
            }
            _ => unimplemented!(),
        }
    }

    fn visit_assignable_element(&mut self, elem: &AssignableElement) -> Instruction {
        // Figure out the target for an assignment
        // We assume that this is the target for
        // the current instruction scope
        match elem {
            AssignableElement::Identifier(ident) => {
                let name = ident.value;
                // Check if this element has been defined
                if self.wast_symbols.lookup(name.into()).is_none() {
                    if self.wast_symbols.depth() == 1 {
                        let sym = self.semantic_symbols.lookup(name.into()).unwrap();
                        self.wast_symbols.define(
                            name.into(),
                            WastSymbol::Global(ValueType::from(sym.ty.clone())),
                        );
                        return Instruction::GlobalSet(name.into(), Box::new(Instruction::Noop));
                    } else {
                        let sym = self.semantic_symbols.lookup(name.into()).unwrap();
                        self.wast_symbols.define(
                            name.into(),
                            WastSymbol::Local(ValueType::from(sym.ty.clone())),
                        );
                        return Instruction::LocalSet(name.into(), Box::new(Instruction::Noop));
                    }
                }
                unreachable!()
            }
        }
    }

    fn visit_single_expression(&mut self, node: &SingleExpression) -> Instruction {
        match node {
            SingleExpression::Additive(exp)
            | SingleExpression::Multiplicative(exp)
            | SingleExpression::Equality(exp)
            | SingleExpression::Bitwise(exp)
            | SingleExpression::Relational(exp) => self.visit_binary_expression(exp),
            SingleExpression::Arguments(exp) => self.visit_argument_expression(exp),
            SingleExpression::Identifier(ident) => self.visit_identifier_expression(ident),
            SingleExpression::Literal(lit) => self.visit_literal(lit),
            SingleExpression::Assignment(exp) => self.visit_assignment_expression(exp),
            SingleExpression::Unary(exp) => self.visit_unary_expression(exp),
            SingleExpression::MemberIndex(exp) => self.visit_member_index(exp),
        }
    }

    fn visit_unary_expression(&mut self, node: &UnaryExpression) -> Instruction {
        let exp = self.visit_single_expression(&node.expr);
        match node.op {
            UnaryOperator::Plus(_) => todo!(),
            UnaryOperator::Minus(_) => {
                Instruction::I32Sub(Box::new(Instruction::I32Const(0)), Box::new(exp))
            }
            UnaryOperator::Not(_) => {
                Instruction::I32Xor(Box::new(exp), Box::new(Instruction::I32Const(-1)))
            }
        }
    }

    fn visit_binary_expression(&mut self, node: &BinaryExpression) -> Instruction {
        let lhs = self.visit_single_expression(&node.left);
        let rhs = self.visit_single_expression(&node.right);
        
        use jswt_common::Typeable;
        let lhs_type = &node.left.defined_type();
        match lhs_type {
            Type::Primitive(p) => match p {
                PrimitiveType::I32 => match node.op {
                    BinaryOperator::Plus(_) => Instruction::I32Add(Box::new(lhs), Box::new(rhs)),
                    BinaryOperator::Minus(_) => Instruction::I32Sub(Box::new(lhs), Box::new(rhs)),
                    BinaryOperator::Mult(_) => Instruction::I32Mul(Box::new(lhs), Box::new(rhs)),
                    BinaryOperator::Equal(_) => Instruction::I32Eq(Box::new(lhs), Box::new(rhs)),
                    BinaryOperator::NotEqual(_) => {
                        Instruction::I32Neq(Box::new(lhs), Box::new(rhs))
                    }
                    BinaryOperator::Div(_) => Instruction::I32Div(Box::new(lhs), Box::new(rhs)),
                    BinaryOperator::And(_) => Instruction::I32And(Box::new(lhs), Box::new(rhs)),
                    BinaryOperator::Or(_) => Instruction::I32Or(Box::new(lhs), Box::new(rhs)),
                    BinaryOperator::Greater(_) => Instruction::I32Gt(Box::new(lhs), Box::new(rhs)),
                    BinaryOperator::GreaterEqual(_) => {
                        Instruction::I32Ge(Box::new(lhs), Box::new(rhs))
                    }
                    BinaryOperator::Less(_) => Instruction::I32Lt(Box::new(lhs), Box::new(rhs)),
                    BinaryOperator::LessEqual(_) => {
                        Instruction::I32Le(Box::new(lhs), Box::new(rhs))
                    }
                    BinaryOperator::Assign(_) => todo!(),
                },
                PrimitiveType::U32 => todo!(),
                PrimitiveType::F32 => match node.op {
                    BinaryOperator::Plus(_) => Instruction::F32Add(Box::new(lhs), Box::new(rhs)),
                    _ => todo!()
                    // BinaryOperator::Minus(_) => Instruction::I32Sub(Box::new(lhs), Box::new(rhs)),
                    // BinaryOperator::Mult(_) => Instruction::I32Mul(Box::new(lhs), Box::new(rhs)),
                    // BinaryOperator::Equal(_) => Instruction::I32Eq(Box::new(lhs), Box::new(rhs)),
                    // BinaryOperator::NotEqual(_) => {
                    //     Instruction::I32Neq(Box::new(lhs), Box::new(rhs))
                    // }
                    // BinaryOperator::Div(_) => Instruction::I32Div(Box::new(lhs), Box::new(rhs)),
                    // BinaryOperator::And(_) => Instruction::I32And(Box::new(lhs), Box::new(rhs)),
                    // BinaryOperator::Or(_) => Instruction::I32Or(Box::new(lhs), Box::new(rhs)),
                    // BinaryOperator::Greater(_) => Instruction::I32Gt(Box::new(lhs), Box::new(rhs)),
                    // BinaryOperator::GreaterEqual(_) => {
                    //     Instruction::I32Ge(Box::new(lhs), Box::new(rhs))
                    // }
                    // BinaryOperator::Less(_) => Instruction::I32Lt(Box::new(lhs), Box::new(rhs)),
                    // BinaryOperator::LessEqual(_) => {
                    //     Instruction::I32Le(Box::new(lhs), Box::new(rhs))
                    // }
                    // BinaryOperator::Assign(_) => todo!(),
                },
                PrimitiveType::Boolean => todo!(),
            },
            Type::Array(_) => todo!(),
            Type::String => todo!(),
            Type::Object => todo!(),
            Type::Function(_, _) => todo!(),
            Type::Void => todo!(),
            Type::Unknown => todo!(),
        }

        // TODO - type check before pushing op
        // match node.op {
        //     BinaryOperator::Plus(_) => Instruction::I32Add(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::Minus(_) => Instruction::I32Sub(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::Mult(_) => Instruction::I32Mul(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::Equal(_) => Instruction::I32Eq(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::NotEqual(_) => Instruction::I32Neq(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::Div(_) => Instruction::I32Div(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::And(_) => Instruction::I32And(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::Or(_) => Instruction::I32Or(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::Greater(_) => Instruction::I32Gt(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::GreaterEqual(_) => Instruction::I32Ge(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::Less(_) => Instruction::I32Lt(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::LessEqual(_) => Instruction::I32Le(Box::new(lhs), Box::new(rhs)),
        //     BinaryOperator::Assign(_) => todo!(),
        // }
    }

    fn visit_identifier_expression(&mut self, node: &IdentifierExpression) -> Instruction {
        let target = node.ident.value;
        if self.wast_symbols.lookup_current(target.into()).is_some() {
            Instruction::LocalGet(target.into())
        } else {
            Instruction::GlobalGet(target.into())
        }
    }

    fn visit_argument_expression(&mut self, node: &ArgumentsExpression) -> Instruction {
        if let SingleExpression::Identifier(ident_exp) = node.ident.borrow() {
            // Push a new instruction scope for the function call
            let instructions = node
                .arguments
                .arguments
                .iter()
                .map(|exp| self.visit_single_expression(exp))
                .collect();

            return Instruction::Call(ident_exp.ident.value.into(), instructions);
        }

        // Other targets for function calls.
        todo!()
    }

    fn visit_literal(&mut self, node: &Literal) -> Instruction {
        match node {
            Literal::String(_) => todo!(),
            Literal::Integer(lit) => Instruction::I32Const(lit.value),
            Literal::Float(lit) => Instruction::F32Const(lit.value),
            Literal::Boolean(lit) => match lit.value {
                // Boolean values in WebAssembly are represented as values of type i32. In a boolean context,
                // such as a br_if condition, any non-zero value is interpreted as true and 0 is interpreted as false.
                true => Instruction::I32Const(1),
                false => Instruction::I32Const(0),
            },
            Literal::Array(lit) => {
                // Synthetic variable to hold the array pointer
                let array_pointer = self.wast_symbols.define_synthetic_local(ValueType::I32);
                let mut instructions = vec![Instruction::LocalSet(
                    array_pointer.clone(),
                    Box::new(Instruction::Call(
                        "arrayNew".into(),
                        vec![Instruction::I32Const(4)], // Size of i32 in bytes
                    )),
                )];

                for element in &lit.elements {
                    // instructions.push(Instruction::I32Store());
                    let value = self.visit_single_expression(element);
                    instructions.push(Instruction::I32Store(
                        Box::new(Instruction::Call(
                            "arrayPush".into(),
                            vec![Instruction::LocalGet(array_pointer.clone())], // Size of i32 in bytes
                        )),
                        Box::new(value),
                    ));
                }

                // Return the array pointer as the result of the expression
                instructions.push(Instruction::LocalGet(array_pointer.clone()));
                Instruction::Complex(instructions)
            }
        }
    }

    fn visit_member_index(&mut self, node: &MemberIndexExpression) -> Instruction {
        let container = self.visit_single_expression(&node.target);
        let index = self.visit_single_expression(&node.index);
        Instruction::Call("arrayAt".into(), vec![container, index])
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use jswt_assert::assert_debug_snapshot;
    use jswt_parser::Parser;
    use jswt_tokenizer::Tokenizer;

    #[test]
    fn test_empty_ast_generates_empty_module() {
        let mut tokenizer = Tokenizer::default();
        tokenizer.enqueue_source_str("test.1", "");
        let ast = Parser::new(&mut tokenizer).parse();

        let mut generator = CodeGenerator::default();
        let actual = generator.generate_module(&ast);
        assert_debug_snapshot!(actual);
    }

    #[test]
    fn test_ast_with_empty_function_generates_module() {
        let mut tokenizer = Tokenizer::default();
        tokenizer.enqueue_source_str("test.1", "function test() {}");
        let ast = Parser::new(&mut tokenizer).parse();
        let mut generator = CodeGenerator::default();
        let actual = generator.generate_module(&ast);
        assert_debug_snapshot!(actual);
    }

    #[test]
    fn test_ast_with_empty_function_with_params_generates_module() {
        let mut tokenizer = Tokenizer::default();
        tokenizer.enqueue_source_str("test.1", "function test(a: i32) {}");
        let ast = Parser::new(&mut tokenizer).parse();

        let mut generator = CodeGenerator::default();
        let actual = generator.generate_module(&ast);
        assert_debug_snapshot!(actual);
    }
    #[test]
    fn test_ast_with_empty_function_with_params_and_return_value_generates_module() {
        let mut tokenizer = Tokenizer::default();
        tokenizer.enqueue_source_str("test.1", "function test(a: i32): i32 {}");
        let ast = Parser::new(&mut tokenizer).parse();

        let mut generator = CodeGenerator::default();
        let actual = generator.generate_module(&ast);
        assert_debug_snapshot!(actual);
    }

    #[test]
    fn test_ast_with_function_containing_return_expression_generates_module() {
        let mut tokenizer = Tokenizer::default();
        tokenizer.enqueue_source_str("test.1", "function test() { return 1 + 2; }");
        let mut parser = Parser::new(&mut tokenizer);
        let ast = parser.parse();

        let mut generator = CodeGenerator::default();
        let actual = generator.generate_module(&ast);
        assert_debug_snapshot!(actual);
    }

    #[test]
    fn test_array_literals_and_array_index_assignment() {
        let mut tokenizer = Tokenizer::default();
        tokenizer.enqueue_source_str(
            "test.1",
            r"
            function test() { 
                let x = [1, 2, 3, 4, 5];
                x[0] = 99;
            }
        ",
        );
        let mut parser = Parser::new(&mut tokenizer);
        let ast = parser.parse();

        assert_eq!(parser.tokenizer_errors().len(), 0);
        assert_eq!(parser.parse_errors().len(), 0);
        let mut generator = CodeGenerator::default();
        let actual = generator.generate_module(&ast);
        assert_debug_snapshot!(actual);
    }
}
