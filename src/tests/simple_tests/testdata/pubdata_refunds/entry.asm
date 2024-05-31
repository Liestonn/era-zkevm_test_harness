    .text
    .file "Test_26"
    .rodata.cst32
    .p2align    5
    .text
    .globl __entry
__entry:
.main:
    ; allocate 20k gas for 2 writes
    ; each write takes about 5k gas
    ; add 20000, r0, r4
    ; 200k
    add 500000, r0, r4
    
    ; perform write via near call
    near_call r4, @inner_s_write, @.panic

    ret.ok r0
inner_s_write:
    ; prepare a 32-bit mask (0xffff..)
    add 1, r0, r10
    shl.s 32, r10, r10
    sub.s 1, r10, r10
    
    ; check pubdata counter (should be equal to 0)
    context.meta r7
    and r10, r7, r7
    sub! 0, r7, r0
    jump.ne @.panic

    ; we'll be writing 13 at slot 25 with a warm refund of 5400
    ; setting r14 to 1 indicates warm storage access
    ; setting r15 to 5400 indicates a refund with that amount
    add 1, r0, r14
    add 5400, r0, r15
    add 25, r0, r1
    add 13, r0, r2
    log.swrite r1, r2, r0

    ; we'll be writing 19 at slot 25
    ; setting r14 to 0 indicates cold storage access
    ; resetting r15 to cleanup last refund
    add 0, r0, r14
    add 0, r0, r15
    add 19, r0, r2
    log.swrite r1, r2, r0

    ; read slot 25 with a warm refund of 1900
    ; setting r14 to 1 indicates warm storage access
    ; setting r15 to 1900 indicates a refund with that amount
    add 1, r0, r14
    add 1900, r0, r15
    log.sread r1, r0, r5

    ; read slot 19
    ; setting r14 to 0 indicates cold storage access
    ; resetting r15 to cleanup last refund
    add 0, r0, r14
    add 0, r0, r15
    add 19, r0, r1
    log.sread r1, r0, r6

    ret.ok r0
.panic:
    ret.panic r0