use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Blueprint {
    pub params: Vec<String>, // e.g. ["T", "U"] extracted from <T, U>
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Fragment {
    pub params: Vec<String>,
    pub body: String,
}

/// Stores definitions for fragments, blueprints, and concrete schemas.
#[derive(Default, Debug)]
pub struct Registry {
    /// @openapi-fragment Name(arg1, arg2)
    pub fragments: HashMap<String, Fragment>,
    /// @openapi<T, U> -> key is Name ("Page")
    pub blueprints: HashMap<String, Blueprint>,
    /// Standard @openapi on structs
    pub schemas: HashMap<String, String>,
    /// Concrete schemas generated from generics (e.g. Page_User)
    pub concrete_schemas: HashMap<String, String>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_fragment(&mut self, name: String, params: Vec<String>, content: String) {
        self.fragments.insert(
            name,
            Fragment {
                params,
                body: content,
            },
        );
    }

    pub fn insert_blueprint(&mut self, name: String, params: Vec<String>, content: String) {
        self.blueprints.insert(
            name,
            Blueprint {
                params,
                body: content,
            },
        );
    }

    pub fn insert_schema(&mut self, name: String, content: String) {
        self.schemas.insert(name, content);
    }
}
