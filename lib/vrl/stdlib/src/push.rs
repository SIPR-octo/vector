use ::value::Value;
use primitive_calling_convention::primitive_calling_convention;
use vrl::prelude::*;

fn push(list: Value, item: Value) -> Resolved {
    let mut list = list.try_array()?;
    list.push(item);
    Ok(list.into())
}

#[derive(Clone, Copy, Debug)]
pub struct Push;

impl Function for Push {
    fn identifier(&self) -> &'static str {
        "push"
    }

    fn parameters(&self) -> &'static [Parameter] {
        &[
            Parameter {
                keyword: "value",
                kind: kind::ARRAY,
                required: true,
            },
            Parameter {
                keyword: "item",
                kind: kind::ANY,
                required: true,
            },
        ]
    }

    fn examples(&self) -> &'static [Example] {
        &[
            Example {
                title: "push item",
                source: r#"push(["foo"], "bar")"#,
                result: Ok(r#"["foo", "bar"]"#),
            },
            Example {
                title: "empty array",
                source: r#"push([], "bar")"#,
                result: Ok(r#"["bar"]"#),
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
        let item = arguments.required("item");

        Ok(Box::new(PushFn { value, item }))
    }

    fn symbol(&self) -> Option<Symbol> {
        Some(Symbol {
            name: "vrl_fn_push",
            address: vrl_fn_push as _,
            uses_context: false,
        })
    }
}

#[derive(Debug, Clone)]
struct PushFn {
    value: Box<dyn Expression>,
    item: Box<dyn Expression>,
}

#[no_mangle]
#[primitive_calling_convention]
extern "C" fn vrl_fn_push(value: Value, item: Value) -> Resolved {
    push(value, item)
}

impl Expression for PushFn {
    fn resolve(&self, ctx: &mut Context) -> Resolved {
        let list = self.value.resolve(ctx)?;
        let item = self.item.resolve(ctx)?;

        push(list, item)
    }

    fn type_def(&self, state: (&state::LocalEnv, &state::ExternalEnv)) -> TypeDef {
        let item = TypeDef::array(BTreeMap::from([(
            0.into(),
            self.item.type_def(state).into(),
        )]));
        self.value
            .type_def(state)
            .restrict_array()
            .merge_append(item)
            .infallible()
    }
}

#[cfg(test)]
mod tests {
    use vector_common::btreemap;

    use super::*;

    test_function![
        push => Push;

        empty_array {
            args: func_args![value: value!([]), item: value!("foo")],
            want: Ok(value!(["foo"])),
            tdef: TypeDef::array(btreemap! {
                Index::from(0) => Kind::bytes(),
            }),
        }

        new_item {
            args: func_args![value: value!([11, false, 42.5]), item: value!("foo")],
            want: Ok(value!([11, false, 42.5, "foo"])),
            tdef: TypeDef::array(btreemap! {
                Index::from(0) => Kind::integer(),
                Index::from(1) => Kind::boolean(),
                Index::from(2) => Kind::float(),
                Index::from(3) => Kind::bytes(),
            }),
        }

        already_exists_item {
            args: func_args![value: value!([11, false, 42.5]), item: value!(42.5)],
            want: Ok(value!([11, false, 42.5, 42.5])),
            tdef: TypeDef::array(btreemap! {
                Index::from(0) => Kind::integer(),
                Index::from(1) => Kind::boolean(),
                Index::from(2) => Kind::float(),
                Index::from(3) => Kind::float(),
            }),
        }
    ];
}
