use primitive_calling_convention::primitive_calling_convention;
use std::any::Any;
use vrl::prelude::{TypeDef as VrlTypeDef, *};

fn type_def(type_def: &VrlTypeDef) -> Resolved {
    let mut tree = type_def.kind().debug_info();

    if type_def.is_fallible() {
        tree.insert("fallible".to_owned(), true.into());
    }

    Ok(tree.into())
}

/// A debug function to print the type definition of an expression at runtime.
///
/// This function is *UNDOCUMENTED* and *UNSTABLE*. It is *NOT* to be advertised
/// to users of Vector, even though it is technically useable by others.
#[derive(Clone, Copy, Debug)]
pub struct TypeDef;

impl Function for TypeDef {
    fn identifier(&self) -> &'static str {
        "type_def"
    }

    fn parameters(&self) -> &'static [Parameter] {
        &[Parameter {
            keyword: "value",
            kind: kind::ANY,
            required: true,
        }]
    }

    fn examples(&self) -> &'static [Example] {
        &[Example {
            title: "return type definition",
            source: r#"type_def(42)"#,
            result: Ok(r#"{ "integer": true }"#),
        }]
    }

    fn compile(
        &self,
        state: (&mut state::LocalEnv, &mut state::ExternalEnv),
        _ctx: &mut FunctionCompileContext,
        mut arguments: ArgumentList,
    ) -> Compiled {
        let value = arguments.required("value");
        let type_def = value.type_def((&*state.0, &*state.1));

        Ok(Box::new(TypeDefFn { type_def }))
    }

    fn compile_argument(
        &self,
        state: (&state::LocalEnv, &state::ExternalEnv),
        _args: &[(&'static str, Option<ResolvedArgument>)],
        _ctx: &mut FunctionCompileContext,
        name: &str,
        expr: Option<&expression::Expr>,
    ) -> CompiledArgument {
        match (name, expr) {
            ("value", expr) => {
                let expr = expr.expect("argument must be provided");
                let type_def = expr.type_def(state);

                Ok(Some(Box::new(type_def) as _))
            }
            _ => Ok(None),
        }
    }

    fn symbol(&self) -> Option<Symbol> {
        Some(Symbol {
            name: "vrl_fn_type_def",
            address: vrl_fn_type_def as _,
            uses_context: false,
        })
    }
}

#[derive(Debug, Clone)]
struct TypeDefFn {
    type_def: VrlTypeDef,
}

impl Expression for TypeDefFn {
    fn resolve(&self, _ctx: &mut Context) -> Resolved {
        type_def(&self.type_def.clone())
    }

    fn type_def(&self, _state: (&state::LocalEnv, &state::ExternalEnv)) -> VrlTypeDef {
        VrlTypeDef::any().infallible()
    }
}

#[no_mangle]
#[primitive_calling_convention]
extern "C" fn vrl_fn_type_def(def: &Box<dyn Any + Send + Sync>) -> Resolved {
    let def = def.downcast_ref::<VrlTypeDef>().unwrap();

    type_def(def)
}
