    .text
    .file "Test_26"
    .rodata.cst32
    .p2align    5
    .text
    .globl __entry
__entry:
.main:
    ; allocate 20k gas for 2 writes and 2 reads
    ; each write takes about 5k gas
    add 20000, r0, r4
    
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
    set_storage_warm(5400)
    add 25, r0, r1
    add 13, r0, r2
    log.swrite r1, r2, r0

    ; we'll be writing 19 at slot 25 with a cold storage refund
    set_storage_cold()
    add 19, r0, r2
    log.swrite r1, r2, r0

    ; read slot 25 with a warm refund of 1900
    set_storage_warm(1900)
    log.sread r1, r0, r5

    ; read slot 19 with a cold storage refund
    set_storage_cold()
    add 19, r0, r1
    log.sread r1, r0, r6

    ret.ok r0
.panic:
    ret.panic r0