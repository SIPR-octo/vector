use ::value::Value;
use primitive_calling_convention::primitive_calling_convention;
use vrl::prelude::*;

fn float(value: Value) -> Resolved {
    match value {
        v @ Value::Float(_) => Ok(v),
        v => Err(format!("expected float, got {}", v.kind()).into()),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Float;

impl Function for Float {
    fn identifier(&self) -> &'static str {
        "float"
    }

    fn parameters(&self) -> &'static [Parameter] {
        &[Parameter {
            keyword: "value",
            kind: kind::ANY,
            required: true,
        }]
    }

    fn examples(&self) -> &'static [Example] {
        &[
            Example {
                title: "valid",
                source: r#"float(3.1415)"#,
                result: Ok("3.1415"),
            },
            Example {
                title: "invalid",
                source: "float!(true)",
                result: Err(
                    r#"function call error for "float" at (0:12): expected float, got boolean"#,
                ),
            },
        ]
    }

    fn compile(
        &self,
        _state: (&mut state::LocalEnv, &mut state::ExternalEnv),
        _ctx: &mut FunctionCompileContext,
        mut arguments: ArgumentList,
    ) -> Compiled {
        let value = arguments.required("value");

        Ok(Box::new(FloatFn { value }))
    }

    fn symbol(&self) -> Option<Symbol> {
        Some(Symbol {
            name: "vrl_fn_float",
            address: vrl_fn_float as _,
            uses_context: false,
        })
    }
}

#[derive(Debug, Clone)]
struct FloatFn {
    value: Box<dyn Expression>,
}

impl Expression for FloatFn {
    fn resolve(&self, ctx: &mut Context) -> Resolved {
        float(self.value.resolve(ctx)?)
    }

    fn type_def(&self, state: (&state::LocalEnv, &state::ExternalEnv)) -> TypeDef {
        let non_float = !self.value.type_def(state).is_float();

        TypeDef::float().with_fallibility(non_float)
    }
}

#[no_mangle]
#[primitive_calling_convention]
extern "C" fn vrl_fn_float(value: Value) -> Resolved {
    float(value)
}
