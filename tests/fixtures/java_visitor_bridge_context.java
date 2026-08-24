package com.test;

// Stand-ins for the generated context record and its discriminant enum, which carry Jackson
// annotations and so cannot be compiled without the Jackson jars on the classpath. Component
// order and types mirror the IR context type the bridge decodes. ~keep
enum NodeKind {
    ELEMENT,
    TEXT
}

record NodeContext(NodeKind kind, String name, long depth, long position, String parent, boolean inline) {}
