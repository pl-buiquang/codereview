(class_declaration name: (identifier) @name) @definition.class
(interface_declaration name: (identifier) @name) @definition.interface
(struct_declaration name: (identifier) @name) @definition.struct
(enum_declaration name: (identifier) @name) @definition.enum
(record_declaration name: (identifier) @name) @definition.class
(delegate_declaration name: (identifier) @name) @definition.type
(method_declaration name: (identifier) @name) @definition.method
(property_declaration name: (identifier) @name) @definition.property
(field_declaration (variable_declaration (variable_declarator name: (identifier) @name))) @definition.field
