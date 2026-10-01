//! Hand-written bindings in the shape that `wasm_junction::bindgen!` will generate.
//!
//! TODO: Replace these bindings with one `wasm_junction::bindgen!` invocation once the macro
//! exists.

/// Bindings for the host-provided users interface.
pub mod users {
    use std::sync::Arc;

    use wasm_junction::{
        BoxFuture, Call, CallContext, HostBound, Provided, Provider, Trap, TypeError, TypedCall,
        Val, Vals,
    };

    /// The fully qualified WIT interface name.
    pub const INTERFACE: &str = "example:greeter/users@0.1.0";

    /// A user returned by the host directory.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct User {
        /// The user's display name.
        pub name: String,
        /// The user's language code.
        pub language: String,
    }

    /// A host implementation of the users interface.
    pub trait Host: HostBound + Sized + 'static {
        /// Looks up a user by numeric id.
        fn lookup(&self, cx: &CallContext, id: u32) -> Option<User>;
    }

    impl<T: Host> Host for Arc<T> {
        fn lookup(&self, cx: &CallContext, id: u32) -> Option<User> {
            self.as_ref().lookup(cx, id)
        }
    }

    /// Wraps a users implementation for registration with an app.
    #[must_use]
    pub fn provider(host: impl Host) -> Provided {
        Provided::new(INTERFACE, HostProvider(host))
    }

    /// Typed view of a `users.lookup` call.
    pub struct Lookup {
        /// The requested user id.
        pub id: u32,
    }

    impl TypedCall for Lookup {
        type Output = Option<User>;
        const INTERFACE: &'static str = INTERFACE;
        const FUNCTION: &'static str = "lookup";

        fn from_vals(values: &[Val]) -> Result<Self, TypeError> {
            let [Val::U32(id)] = values else {
                return Err(TypeError::new("users.lookup expected one u32"));
            };
            Ok(Self { id: *id })
        }

        fn into_vals(self) -> Vals {
            vec![Val::U32(self.id)]
        }

        fn output(value: Self::Output) -> Vals {
            vec![Val::Option(value.map(user_into_val).map(Box::new))]
        }

        fn decode_output(values: &[Val]) -> Result<Self::Output, TypeError> {
            let [Val::Option(value)] = values else {
                return Err(TypeError::new("users.lookup expected one option<user>"));
            };
            value.as_deref().map(user_from_val).transpose()
        }
    }

    struct HostProvider<T>(T);

    impl<T: Host> Provider for HostProvider<T> {
        fn call<'a>(
            &'a self,
            cx: &'a CallContext,
            call: Call,
        ) -> BoxFuture<'a, Result<Vals, Trap>> {
            Box::pin(async move {
                if call.function.as_ref() != "lookup" {
                    return Err(Trap::new(format!(
                        "unknown users function `{}`",
                        call.function
                    )));
                }
                let Lookup { id } = Lookup::from_vals(&call.args)?;
                Ok(Lookup::output(self.0.lookup(cx, id)))
            })
        }
    }

    fn user_into_val(user: User) -> Val {
        Val::Record(vec![
            ("name".to_owned(), Val::String(user.name)),
            ("language".to_owned(), Val::String(user.language)),
        ])
    }

    fn user_from_val(value: &Val) -> Result<User, TypeError> {
        let Val::Record(fields) = value else {
            return Err(TypeError::new("expected user record"));
        };
        let [
            (name_field, Val::String(name)),
            (language_field, Val::String(language)),
        ] = fields.as_slice()
        else {
            return Err(TypeError::new("expected user name and language"));
        };
        if name_field != "name" || language_field != "language" {
            return Err(TypeError::new("expected user name and language fields"));
        }
        Ok(User {
            name: name.clone(),
            language: language.clone(),
        })
    }
}
