; Definition query. Each pattern captures the declared identifier as @name and the whole
; declaration as @definition.<kind>.

(method name: (_) @name) @definition.method
(singleton_method name: (_) @name) @definition.method

(class name: [(constant) @name (scope_resolution name: (_) @name)]) @definition.class
(module name: [(constant) @name (scope_resolution name: (_) @name)]) @definition.module

; FOO = ..., Foo = Struct.new(...)
(assignment left: (constant) @name) @definition.constant

; Declarations made by calling a class macro with symbol arguments.
((call
   method: (identifier) @_macro
   arguments: (argument_list (simple_symbol) @name)) @definition.macro
 (#match? @_macro "^(attr_reader|attr_writer|attr_accessor|scope|define_method|has_many|has_one|belongs_to|has_and_belongs_to_many|delegate)$"))
