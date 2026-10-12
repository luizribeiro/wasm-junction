(component
  (core module $module
    (memory (export "memory") 1)

    (func $unreachable (export "unreachable")
      unreachable
    )

    (func $out-of-bounds (export "out-of-bounds")
      i32.const 65536
      i32.const 0
      i32.const 1
      memory.copy
    )

    (func $stack-overflow (export "stack-overflow")
      call $stack-overflow
    )

    (func $integer-divide-by-zero (export "integer-divide-by-zero")
      i32.const 1
      i32.const 0
      i32.div_s
      drop
    )
  )

  (core instance $instance (instantiate $module))
  (func $unreachable (canon lift (core func $instance "unreachable")))
  (func $out-of-bounds (canon lift (core func $instance "out-of-bounds")))
  (func $stack-overflow (canon lift (core func $instance "stack-overflow")))
  (func $integer-divide-by-zero
    (canon lift (core func $instance "integer-divide-by-zero")))

  (instance $probe
    (export "unreachable" (func $unreachable))
    (export "out-of-bounds" (func $out-of-bounds))
    (export "stack-overflow" (func $stack-overflow))
    (export "integer-divide-by-zero" (func $integer-divide-by-zero))
  )
  (export "example:traps/probe@0.1.0" (instance $probe))
)
