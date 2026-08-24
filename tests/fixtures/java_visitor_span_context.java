package com.test;

import java.util.List;

// Stand-in for the generated context record, which carries Jackson annotations and so cannot be
// compiled without the Jackson jars on the classpath. Component order and types mirror the IR
// context type the bridge decodes, including the two the C struct cannot carry. ~keep
record SpanContext(
        String label,
        byte severity,
        boolean active,
        short offset,
        String note,
        double weight,
        List<String> tags) {}
