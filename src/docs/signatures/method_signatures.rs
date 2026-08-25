use super::*;
use crate::docs::naming::func_name;

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------

#[test]
fn test_render_method_signature_python_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "def get_text(self, page: int) -> str");
}

#[test]
fn test_render_method_signature_python_async() {
    let method = make_method("process", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Document", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "def process(self) -> str");
}

#[test]
fn test_render_method_signature_python_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "@staticmethod\ndef create(name: str) -> Document");
}

#[test]
fn test_render_method_signature_python_optional_return() {
    let method = make_method(
        "find",
        vec![make_param("query", TypeRef::String, false)],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "def find(self, query: str) -> str | None");
}

#[test]
fn test_render_method_signature_python_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Python, TEST_PREFIX);
    assert_eq!(sig, "def parse(self, source: str) -> Ast");
}

#[test]
fn test_render_method_signature_node_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "getText(page: number): string");
}

#[test]
fn test_render_method_signature_node_async() {
    let method = make_method("process", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Document", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "process(): Promise<string>");
}

#[test]
fn test_render_method_signature_node_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "static create(name: string): Document");
}

#[test]
fn test_render_method_signature_node_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "find(): string | null");
}

#[test]
fn test_render_method_signature_node_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Node, TEST_PREFIX);
    assert_eq!(sig, "parse(source: string): Ast");
}

#[test]
fn test_render_method_signature_rust_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub fn get_text(&self, page: u32) -> String");
}

#[test]
fn test_render_method_signature_rust_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub async fn fetch(&self) -> String");
}

#[test]
fn test_render_method_signature_rust_static() {
    let method = make_method(
        "new",
        vec![make_ref_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub fn new(name: &str) -> Document");
}

#[test]
fn test_render_method_signature_rust_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::Named("Node".to_string()))),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Tree", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub fn find(&self) -> Option<Node>");
}

#[test]
fn test_render_method_signature_rust_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_ref_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Rust, TEST_PREFIX);
    assert_eq!(sig, "pub fn parse(&self, source: &str) -> Result<Ast, ParseError>");
}

#[test]
fn test_render_method_signature_go_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *Document) GetText(page uint32) string");
}

#[test]
fn test_render_method_signature_go_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *Client) Fetch() string");
}

#[test]
fn test_render_method_signature_go_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *Corpus) Find() *string");
}

#[test]
fn test_render_method_signature_go_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Go, TEST_PREFIX);
    // ~keep A Named return is pointer-wrapped in real Go regardless of fallibility -- see
    // signatures.rs's `go_return_type` for the source citation (backends/go/type_map.rs's
    // `go_optional_type`, which every Go method/function return routes a Named type through).
    assert_eq!(sig, "func (o *Parser) Parse(source string) (*Ast, error)");
}

/// ~keep A static Go method is a free function `func {Type}{Method}(...)`, never a method
/// with a receiver -- see `method_signature_static.jinja` (backends/go/templates/):
/// `"func {{ receiver_type }}{{ method_name }}({{ params }}){{ return_type }} {"`. The
/// expected string here is reconstructed from the same `type_name`/`func_name` building
/// blocks the renderer itself uses, mirroring the template's exact concatenation, rather
/// than a hand-picked literal that could independently drift from what the template
/// actually says -- the same discipline as the C signature-vs-example cross-checks.
#[test]
fn test_render_method_signature_go_static_method_has_no_receiver() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Go, TEST_PREFIX);

    let receiver_type = type_name("Document", Language::Go, TEST_PREFIX);
    let method_name = func_name("create", Language::Go, TEST_PREFIX);
    // ~keep A Named return is pointer-wrapped in real Go -- see `go_return_type`.
    let expected = format!("func {receiver_type}{method_name}(name string) *Document");
    assert_eq!(sig, expected);
    assert!(
        !sig.contains("(o *"),
        "a static Go method is a free function, not a method with a receiver: {sig}"
    );
}

#[test]
fn test_render_method_signature_go_static_vs_instance_differ_only_in_receiver_clause() {
    // The static and instance renderings of the *same* method must differ only in whether
    // a receiver clause is present -- not in name, params, or return type.
    let instance_method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let mut static_method = instance_method.clone();
    static_method.is_static = true;

    let instance_sig = render_method_signature(&instance_method, "Document", Language::Go, TEST_PREFIX);
    let static_sig = render_method_signature(&static_method, "Document", Language::Go, TEST_PREFIX);

    assert_eq!(instance_sig, "func (o *Document) GetText(page uint32) string");
    assert_eq!(static_sig, "func DocumentGetText(page uint32) string");

    let call_tail = "GetText(page uint32) string";
    assert!(instance_sig.ends_with(call_tail) && static_sig.ends_with(call_tail));
}

#[test]
fn test_render_method_signature_go_error_type_unit_return() {
    let method = make_method("save", vec![], TypeRef::Unit, false, false, Some("IoError"));
    let sig = render_method_signature(&method, "File", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *File) Save() error");
}

/// ~keep A Named return is pointer-wrapped in real Go unconditionally -- `gen_method_wrapper`
/// (backends/go/gen_bindings/methods.rs) routes any non-Primitive/Duration/String/Char/Path
/// return through `go_optional_type` regardless of whether the method can fail. This method
/// has no `error_type`, isolating the pointer-wrapping from the separate `(T, error)`
/// tuple-wrapping behavior already covered by `test_render_method_signature_go_with_error_type`.
#[test]
fn test_render_method_signature_go_named_return_is_pointer_wrapped_even_when_infallible() {
    let method = make_method(
        "current",
        vec![],
        TypeRef::Named("Session".to_string()),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Client", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *Client) Current() *Session");
}

#[test]
fn test_render_method_signature_go_non_named_return_is_unaffected_by_pointer_wrapping() {
    // Optional<String> already renders as `*string` via doc_type's own Go Optional
    // handling -- `go_return_type` must not double the pointer or otherwise touch it.
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Go, TEST_PREFIX);
    assert_eq!(sig, "func (o *Corpus) Find() *string");
}

#[test]
fn test_render_method_signature_ruby_sync_with_params() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def get_text(page)");
}

#[test]
fn test_render_method_signature_ruby_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def self.create(name)");
}

#[test]
fn test_render_method_signature_ruby_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def fetch()");
}

#[test]
fn test_render_method_signature_ruby_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def find()");
}

#[test]
fn test_render_method_signature_ruby_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::Ruby, TEST_PREFIX);
    assert_eq!(sig, "def parse()");
}

#[test]
fn test_render_method_signature_php_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public function getText(int $page): string");
}

#[test]
fn test_render_method_signature_php_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public static function create(string $name): Document");
}

#[test]
fn test_render_method_signature_php_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public function find(): ?string");
}

#[test]
fn test_render_method_signature_php_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public function fetch(): string");
}

#[test]
fn test_render_method_signature_php_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::Php, TEST_PREFIX);
    assert_eq!(sig, "public function parse(): string");
}

/// ~keep Every generated Java instance method -- fallible or not -- crosses the FFI boundary
/// through `emit_instance_method_header` (`gen_bindings/types/opaque/instance.rs`), which
/// appends `" throws "` + `symbols.exception_class` (`format!("{main_class}Exception")`)
/// unconditionally except for `clone` (see the dedicated clone test below). `main_class` is
/// `resolve_main_class`: the crate name PascalCased, with an `Rs` suffix unless it already
/// ends in `Rs`. The FFI crossing itself can fail (marshaling, allocation) even when the
/// wrapped Rust method returns a bare `T`, not a `Result` -- so `method.error_type` (a fact
/// about the *core* Rust signature) is the wrong oracle for this clause; it must always be
/// present, and it must always name the FFI exception, never the domain error type. This is
/// also why every `**Example:**` block that doesn't `try`/`catch` this checked exception
/// fails to compile -- see the docs-writer report for that half of the fix.
#[test]
fn test_render_method_signature_java_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public String getText(int page) throws HtmRsException");
}

/// ~keep Static factory methods go through a different emitter
/// (`emit_static_factory_header`, `gen_bindings/types/opaque/extended.rs`) but declare the
/// same unconditional `throws {main_class}Exception` -- the `is_static` branch must not be
/// the thing that turns throws off.
#[test]
fn test_render_method_signature_java_static() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public static Document create(String name) throws HtmRsException");
}

#[test]
fn test_render_method_signature_java_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Java, TEST_PREFIX);
    // ~keep The emitted Java facade renders an `Option<T>` return as `@Nullable T` and unwraps at
    // the boundary -- only the internal `…Rs` FFI layer returns `Optional<T>`. `doc_type` now
    // delegates to `render_nullable_type`, the same function the facade codegen calls.
    assert_eq!(sig, "public @Nullable String find() throws HtmRsException");
}

#[test]
fn test_render_method_signature_java_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public String fetch() throws HtmRsException");
}

/// ~keep A domain `error_type` must NOT leak into the throws clause -- the real emitter never
/// names it. Guards against a fix that reads `throws {}` from `method.error_type` when present
/// and only falls back to the FFI exception otherwise (still wrong in both directions: it
/// would have passed the old test's literal string, but not the real generated code).
#[test]
fn test_render_method_signature_java_with_error_type() {
    let method = make_method(
        "parse",
        vec![make_param("source", TypeRef::String, false)],
        TypeRef::Named("Ast".to_string()),
        false,
        false,
        Some("ParseError"),
    );
    let sig = render_method_signature(&method, "Parser", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public Ast parse(String source) throws HtmRsException");
}

/// ~keep The one documented exception to the unconditional throws rule: `emit_instance_method_header`
/// checks `if method.name != "clone"` before appending the throws clause -- `clone()` never
/// declares it. This is the positive control in the other direction: a fix that adds `throws`
/// to every method with no exceptions would pass every test above and still document a
/// non-existent checked exception on `clone()`.
#[test]
fn test_render_method_signature_java_clone_has_no_throws() {
    let method = make_method(
        "clone",
        vec![],
        TypeRef::Named("Document".to_string()),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Java, TEST_PREFIX);
    assert_eq!(sig, "public Document clone()");
    assert!(
        !sig.contains("throws"),
        "clone() never declares a checked exception: {sig}"
    );
}

#[test]
fn test_render_method_signature_csharp_sync_with_params_and_return() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public string GetText(uint page)");
}

#[test]
fn test_render_method_signature_csharp_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public async Task<string> FetchAsync()");
}

#[test]
fn test_render_method_signature_csharp_async_already_suffixed() {
    let method = make_method("fetch_async", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public async Task<string> FetchAsync()");
}

#[test]
fn test_render_method_signature_csharp_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public string? Find()");
}

#[test]
fn test_render_method_signature_csharp_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::Csharp, TEST_PREFIX);
    assert_eq!(sig, "public string Parse()");
}

/// ~keep An instance Elixir method takes the struct as an explicit leading `obj` parameter --
/// `conversions.rs`'s rustler codegen (`gen_bindings/helpers/conversions.rs`) pushes
/// `def_args.push("obj".to_string())` whenever `method.receiver.is_some()`, and
/// `elixir_opaque_method_wrapper.ex.jinja` emits `def {{ method_name }}({{ def_args }})` from
/// that list verbatim -- there is no implicit `self` the way Ruby/Python have one. Documenting
/// `def get_text(page)` for an instance method is not valid Elixir: nothing binds `page` to a
/// `%Document{}` to dispatch through, and the arity is one lower than the real function.
#[test]
fn test_render_method_signature_elixir_sync_with_params() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Elixir, TEST_PREFIX);
    assert_eq!(sig, "def get_text(obj, page)");
}

#[test]
fn test_render_method_signature_elixir_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::Elixir, TEST_PREFIX);
    assert_eq!(sig, "def fetch(obj)");
}

#[test]
fn test_render_method_signature_elixir_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::Elixir, TEST_PREFIX);
    assert_eq!(sig, "def find(obj)");
}

#[test]
fn test_render_method_signature_elixir_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::Elixir, TEST_PREFIX);
    assert_eq!(sig, "def parse(obj)");
}

/// ~keep The mirror image of the Go receiver contract test above: a *static* Elixir function
/// (`method.receiver.is_none()`, e.g. an alternate constructor) never gets the `obj` struct
/// parameter -- `conversions.rs` only pushes it when `method.receiver.is_some()`. A fix that
/// adds `obj` unconditionally (rather than gating on `is_static`) would pass every test above
/// and still document a receiver on a static method that has none.
#[test]
fn test_render_method_signature_elixir_static_method_has_no_receiver() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Elixir, TEST_PREFIX);
    assert_eq!(sig, "def create(name)");
    assert!(
        !sig.contains("obj"),
        "a static Elixir function has no struct receiver: {sig}"
    );
}

/// ~keep Same discipline as `test_render_method_signature_go_static_vs_instance_differ_only_in_receiver_clause`:
/// the static and instance renderings of the *same* method must differ only in whether the
/// leading `obj` parameter is present -- not in name, remaining params, or return type.
#[test]
fn test_render_method_signature_elixir_static_vs_instance_differ_only_in_receiver() {
    let instance_method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let mut static_method = instance_method.clone();
    static_method.is_static = true;

    let instance_sig = render_method_signature(&instance_method, "Document", Language::Elixir, TEST_PREFIX);
    let static_sig = render_method_signature(&static_method, "Document", Language::Elixir, TEST_PREFIX);

    assert_eq!(instance_sig, "def get_text(obj, page)");
    assert_eq!(static_sig, "def get_text(page)");
}

#[test]
fn test_render_method_signature_r_sync_with_params() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::R, TEST_PREFIX);
    assert_eq!(sig, "get_text(page)");
}

#[test]
fn test_render_method_signature_r_async() {
    let method = make_method("fetch", vec![], TypeRef::String, true, false, None);
    let sig = render_method_signature(&method, "Client", Language::R, TEST_PREFIX);
    assert_eq!(sig, "fetch()");
}

#[test]
fn test_render_method_signature_r_optional_return() {
    let method = make_method(
        "find",
        vec![],
        TypeRef::Optional(Box::new(TypeRef::String)),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Corpus", Language::R, TEST_PREFIX);
    assert_eq!(sig, "find()");
}

#[test]
fn test_render_method_signature_r_with_error_type() {
    let method = make_method("parse", vec![], TypeRef::String, false, false, Some("ParseError"));
    let sig = render_method_signature(&method, "Parser", Language::R, TEST_PREFIX);
    assert_eq!(sig, "parse()");
}

#[test]
fn test_render_method_signature_wasm_sync() {
    let method = make_method(
        "get_text",
        vec![make_param("page", TypeRef::Primitive(PrimitiveType::U32), false)],
        TypeRef::String,
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Wasm, TEST_PREFIX);
    assert_eq!(sig, "getText(page: number): string");
}

#[test]
fn test_render_method_signature_wasm_static() {
    let method = make_method(
        "create",
        vec![],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::Wasm, TEST_PREFIX);
    assert_eq!(sig, "static create(): Document");
}

// ---------------------------------------------------------------------------
// ~keep Regression coverage for task #67: C method symbols must fold in the owning type
// (`{prefix}_{type_snake}_{method}`, matching `gen_method_wrapper` /
// `gen_streaming_method_wrapper` in backends/ffi/gen_bindings/functions/orchestration.rs),
// and a non-static method's C signature must declare the leading `this: AlefHandle`
// receiver the backend always emits. Before this fix, the docs published `{prefix}_{method}`
// with no receiver parameter -- a symbol that occurs zero times in the emitted header, and
// even if it did, one the caller could not invoke without an extra argument.
// ---------------------------------------------------------------------------

/// The full C method signature for a non-static method, derived from source rather than a
/// hand-picked literal: `HTMAlefHandle` is `type_name(FFI_HANDLE_TYPE_NAME, C, TEST_PREFIX)`
/// and `htm_converter_convert` is `method_name("Converter", "convert", C, TEST_PREFIX)`.
#[test]
fn test_render_method_signature_c_non_static_declares_this_receiver() {
    let method = make_method(
        "convert",
        vec![make_param("options", TypeRef::Named("ParseOptions".to_string()), false)],
        TypeRef::Named("ConversionResult".to_string()),
        false,
        false,
        None,
    );
    let sig = render_method_signature(&method, "Converter", Language::C, TEST_PREFIX);
    assert_eq!(
        sig,
        "HTMAlefHandle htm_converter_convert(HTMAlefHandle this, HTMAlefHandle options);"
    );
}

/// The mirror image: a static method has no receiver at all -- `gen_method_wrapper` only
/// pushes the `this` parameter `if !method.is_static`.
#[test]
fn test_render_method_signature_c_static_method_has_no_this_receiver() {
    let method = make_method(
        "create",
        vec![make_param("name", TypeRef::String, false)],
        TypeRef::Named("Document".to_string()),
        false,
        true,
        None,
    );
    let sig = render_method_signature(&method, "Document", Language::C, TEST_PREFIX);
    assert_eq!(sig, "HTMAlefHandle htm_document_create(const char* name);");
    assert!(
        !sig.contains("this"),
        "a static method has no instance to receive, so it must not declare `this`: {sig}"
    );
}

/// Neither surface may re-derive the C method symbol independently -- both must equal
/// `naming::method_name`'s output, not merely equal each other (two independently wrong
/// formulas could still agree with one another while both being wrong).
#[test]
fn test_c_signature_and_example_agree_on_method_symbol() {
    let method = make_method(
        "convert",
        vec![make_param("options", TypeRef::Named("ParseOptions".to_string()), false)],
        TypeRef::Named("ConversionResult".to_string()),
        false,
        false,
        None,
    );
    let expected_symbol = method_name("Converter", &method.name, Language::C, TEST_PREFIX);
    assert_eq!(expected_symbol, "htm_converter_convert");

    let signature = render_method_signature(&method, "Converter", Language::C, TEST_PREFIX);
    assert!(
        signature.starts_with(&format!("HTMAlefHandle {expected_symbol}(")),
        "signature: {signature}"
    );

    let example = crate::docs::examples::render_method_example(&method, "Converter", Language::C, TEST_PREFIX);
    assert!(
        example.contains(&format!("{expected_symbol}(instance")),
        "example must call the same symbol the signature declares: {example}"
    );
}

/// This is the assertion that would have caught the pre-existing bug on its own: the example
/// passed a receiver (`instance`) as the first call argument, but the signature declared no
/// `this` parameter to receive it -- one argument more than the documented function takes.
/// Extracts each surface's parameter/argument list independently and compares arity, not text.
#[test]
fn test_c_signature_and_example_agree_on_arity_for_non_static_method() {
    let method = make_method(
        "convert",
        vec![make_param("options", TypeRef::Named("ParseOptions".to_string()), false)],
        TypeRef::Named("ConversionResult".to_string()),
        false,
        false,
        None,
    );
    let name = method_name("Converter", &method.name, Language::C, TEST_PREFIX);

    let signature = render_method_signature(&method, "Converter", Language::C, TEST_PREFIX);
    let example = crate::docs::examples::render_method_example(&method, "Converter", Language::C, TEST_PREFIX);

    let sig_arity = c_paren_items(&signature, &format!("{name}("));
    let call_arity = c_paren_items(&example, &format!("{name}("));

    assert_eq!(sig_arity, 2, "expected `this` plus one declared param: {signature}");
    assert_eq!(
        sig_arity, call_arity,
        "signature declares {sig_arity} param(s) but the example passes {call_arity} arg(s) -- \
         signature: {signature}\nexample: {example}"
    );
}

/// Extract the comma-separated item count inside the first `marker(...)` occurrence in `text`.
/// Local to this module: the fixtures used here never nest parentheses inside a param type or
/// argument, so a plain first-`)`-wins scan is sufficient and keeps the parsing honest rather
/// than reusing a general-purpose parser the production code doesn't have either.
fn c_paren_items(text: &str, marker: &str) -> usize {
    let start = text
        .find(marker)
        .unwrap_or_else(|| panic!("`{marker}` not found in: {text}"))
        + marker.len();
    let rest = &text[start..];
    let end = rest
        .find(')')
        .unwrap_or_else(|| panic!("no closing `)` after `{marker}` in: {text}"));
    let items = rest[..end].trim();
    if items.is_empty() { 0 } else { items.split(',').count() }
}
