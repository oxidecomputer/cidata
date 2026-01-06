; This Source Code Form is subject to the terms of the Mozilla Public
; License, v. 2.0. If a copy of the MPL was not distributed with this
; file, You can obtain one at https://mozilla.org/MPL/2.0/.

; This has absolutely nothing to do with this crate's ability to
; make correct cidata file systems, but it is what we are filling
; the 448-byte code section of the boot sector with that is otherwise
; unused. It's a silly demo / easter egg.

; An assembled copy is stored in the repo for use by `include_bytes!`.
; You can assemble this with:
; $ nasm -f bin boot.asm
; and test it with:
; $ qemu-system-x86_64 -drive file=boot,format=raw

; The cursive bitmap font is a modified version of "Kvalligraphy" by
; kva64, found at https://kva64.itch.io/kvalligraphy and used
; under the Creative Commons Attribution 4.0 International License:
; http://creativecommons.org/licenses/by/4.0/

        bits 16
        org 7c00h

boot_jmp:
        jmp start
        nop
        %if ($-$$) != 3
        %error boot_jmp is wrong
        %endif

        times 62 - ($-$$) db 0         ; FAT BIOS Parameter Block lives here.

start:
; These are not required by QEMU but we include them for maximum
; theoretical compatibility.
        xor cx, cx                     ; Ensure CX=0.
        mov ds, cx                     ; Ensure DS=0.
        mov es, cx                     ; Ensure ES=0.
        cld                            ; Ensure movsb increments SI and DI.

; Unpack our font table into RAM.
        mov si, font
        mov di, end_boot_sector
unpack_start:
        lodsb                          ; Load DS:SI to AL.
        and al, al                     ; If we read 0, the font table is over.
        jz unpack_done
        aam 16                         ; AL now bytes to advance before coyping.
        xchg cl, ah                    ; CX now number of bytes to copy.
        jz unpack_repeat               ; Jump if AL=0 (set by AAM; XCHG does not affect flags).
        add di, ax                     ; Advance DI by AX.
        rep movsb                      ; Move CX bytes from DS:SI to ES:DI.
        jmp unpack_start
unpack_repeat:
; In this mode, CX is the number of times to repeat the next byte.
        lodsb                          ; Load DS:SI to AL.
        rep stosb                      ; Fill CX bytes at ES:DI with AL.
        jmp unpack_start
unpack_done:

; INT 10h AH=00h AL=03h: 80x25 16-color text mode (clears the display)
        mov al, 03h                    ; AH is already zeroed
        int 10h

; INT 10h AH=11h AL=00h user character load
        mov ax, 1100h
        mov bx, 1000h                  ; BH=16 bytes per character, BL=0 character table
        %define char_count 31
        mov cl, char_count             ; CX count of characters in table (CH is already zeroed)
; We map characters in the box drawing area so that the 8th column is
; duplicated into the 9th column, because cursive.
        %define char_start 0xC0
        mov dx, char_start             ; DX codepoint of first character defined
        mov bp, end_boot_sector        ; ES:BP pointer to font table
        int 10h

        xor bx, bx
        mov dx, 0A26h
        mov si, floppy1
        call writestr                  ; now SI=floppy2
        inc dh
        call writestr                  ; now SI=text
        mov dx, 0E17h
        call writestr

        cli
        hlt

; Move the cursor and print a null-terminated string
; DH row
; DL column
; DS:SI string to print
; precondition: BH=0
writestr:
        mov ah, 02h                    ; INT 10h AH=02h set cursor position
        int 10h
        mov ah, 0Eh
writestr_loop:
        lodsb                          ; load DS:SI to AL and increment SI
        test al, al                    ; jump if AL=0
        jz writestr_end
        int 10h                        ; INT 10h AH=0Eh print AL to teletype
        jmp writestr_loop
writestr_end:
        ret

; Our font table. Characters are nominally 16 bytes long, but the large
; number of 00h bytes lends itself well to a simple packing algorithm.
; Each character is preceded by a control byte which lists the number of
; bytes to advance in RAM in the least significant half and the number
; of bytes to copy in the most significant half. 00h indicates the end
; of the table.
font:
        %assign char_idx char_start
        %assign char_pad_prev 0
        %macro char 3                  ; name, pad_start, len
        %assign %1 char_idx
        %assign char_idx char_idx + 1
        db (%3 << 4) | (%2 + char_pad_prev)
        %assign char_pad_prev 16 - %2 - %3
        %endmacro

        char a, 6, 5
        db 0b00011100
        db 0b00100100
        db 0b11000100
        db 0b01011011
        db 0b00101100

        char a_start, 6, 5
        db 0b00011100
        db 0b00100100
        db 0b01000100
        db 0b01011011
        db 0b00101100

        char b, 3, 8
        db 0b00001100
        db 0b00010100
        db 0b00010100
        db 0b00101000
        db 0b00111000
        db 0b01100100
        db 0b11001011
        db 0b01110000

        char C, 3, 8
        db 0b00001100
        db 0b00010010
        db 0b00100010
        db 0b00100000
        db 0b01000000
        db 0b01000001
        db 0b01000110
        db 0b01111000

        char c, 6, 5
        db 0b00011000
        db 0b00101000
        db 0b11000001
        db 0b01001110
        db 0b01110000

        char c_cedilla, 6, 8
        db 0b00011000
        db 0b00101000
        db 0b11000001
        db 0b01001110
        db 0b01110000
        db 0b00010000
        db 0b00010000
        db 0b00100000

        char d, 3, 8
        db 0b00000100
        db 0b00000100
        db 0b00001000
        db 0b00001000
        db 0b01110000
        db 0b10010001
        db 0b10110110
        db 0b01011000

        char e, 6, 5
        db 0b00011100
        db 0b00100100
        db 0b11111000
        db 0b01000111
        db 0b00111000

        char e_end, 6, 5
        db 0b00011100
        db 0b00100100
        db 0b11111000
        db 0b01000110
        db 0b00111000

        char apos_e, 2, 9
        db 0b01000000
        db 0b01000000
        db 0b10000000
        db 0b00000000
        db 0b00011100
        db 0b00100100
        db 0b11111000
        db 0b01000111
        db 0b00111000

        char i, 3, 8
        db 0b00000100
        db 0b00000000
        db 0b00000000
        db 0b00010000
        db 0b11100000
        db 0b00100000
        db 0b01000011
        db 0b00111100

        char l, 3, 8
        db 0b00011000
        db 0b00101000
        db 0b00101000
        db 0b00110000
        db 0b00100000
        db 0b11000001
        db 0b01000110
        db 0b00111000

        char m, 6, 5
        db 0b01011110
        db 0b01101010
        db 0b10010100
        db 0b10000101
        db 0b10000010

        char n, 6, 5
        db 0b00010100
        db 0b00111010
        db 0b11100100
        db 0b01001001
        db 0b01001110

        char o, 6, 5
        db 0b00011100
        db 0b00100110
        db 0b01000101
        db 0b11001000
        db 0b00110000

        char p, 6, 9
        db 0b00011100
        db 0b00010010
        db 0b00100010
        db 0b00100101
        db 0b00111110
        db 0b01000000
        db 0b01000000
        db 0b10000000
        db 0b10000000

        char q, 6, 9
        db 0b00011100
        db 0b11100100
        db 0b01001000
        db 0b01001011
        db 0b00111100
        db 0b00001110
        db 0b00001010
        db 0b00001010
        db 0b00001100

        char r, 6, 5
        db 0b01101000
        db 0b10110000
        db 0b00010000
        db 0b00010111
        db 0b00011000

        char s, 6, 5
        db 0b00001000
        db 0b00010100
        db 0b11100011
        db 0b00000100
        db 0b00011000

        char s_end, 6, 5
        db 0b00001000
        db 0b00010100
        db 0b01100010
        db 0b10000100
        db 0b00011000

        char t, 3, 8
        db 0b00000100
        db 0b00001000
        db 0b00001000
        db 0b00111100
        db 0b11010000
        db 0b00100000
        db 0b01011100
        db 0b01100000

        char u, 6, 5
        db 0b00100100
        db 0b01001000
        db 0b01010000
        db 0b10010011
        db 0b01101100

        char period, 10, 1
        db 0b00100000
        db char_pad_prev << 4, 0       ; final padding

; The floppy disk sprites use a different compression scheme where the
; control byte's least significant half is 0 and the most significant
; half is the number of times to repeat the next byte.
        %macro floppychar 1
        %assign %1 char_idx
        %assign char_idx char_idx + 1
        %endmacro

        floppychar floppy1_1
        db 1 << 4, 0b00011111
        db 9 << 4, 0b00100010
        db 1 << 4, 0b00100001
        db 2 << 4, 0b00100000
        db 1 << 4, 0b00100001
        db 2 << 4, 0b00100010

        floppychar floppy1_2
        db 1 << 4, 0b11111111
        db 9 << 4, 0b01000000
        db 1 << 4, 0b11111111
        db 2 << 4, 0b00000000
        db 1 << 4, 0b11111111
        db 2 << 4, 0b00000000

        floppychar floppy1_3
        db 1 << 4, 0b11111111
        db 1 << 4, 0b00000010
        db 7 << 4, 0b01110010
        db 1 << 4, 0b00000010
        db 1 << 4, 0b11111100
        db 2 << 4, 0b00000000
        db 1 << 4, 0b11111111
        db 2 << 4, 0b00000000

        floppychar floppy1_4
        db 1 << 4, 0b11100000
        db 1 << 4, 0b00010000
        db 1 << 4, 0b00001000
        db 1 << 4, 0b00000100
        db 9 << 4, 0b00000010
        db 1 << 4, 0b11000010
        db 2 << 4, 0b00100010

        floppychar floppy2_1
        db 15 << 4, 0b00100010
        db 01 << 4, 0b00011111

        floppychar floppy2_2
        db 2 << 4, 0b00000000
        db 1 << 4, 0b00000001
        db 1 << 4, 0b00000011
        db 1 << 4, 0b00000111
        db 1 << 4, 0b00010111
        db 1 << 4, 0b00110111
        db 1 << 4, 0b00111011
        db 2 << 4, 0b00111111
        db 1 << 4, 0b00011111
        db 1 << 4, 0b00001111
        db 3 << 4, 0b00000000
        db 1 << 4, 0b11111111

        floppychar floppy2_3
        db 2 << 4, 0b00000000
        db 1 << 4, 0b11000000
        db 1 << 4, 0b11100000
        db 3 << 4, 0b11110000
        db 1 << 4, 0b11101100
        db 1 << 4, 0b11011110
        db 2 << 4, 0b11111110
        db 1 << 4, 0b11111100
        db 3 << 4, 0b00000000
        db 1 << 4, 0b11111111

        floppychar floppy2_4
        db 15 << 4, 0b00100010
        db 01 << 4, 0b11111100

        %if char_idx - char_start != char_count
        %error char_count is wrong
        %endif
        db 0

floppy1:
        db floppy1_1, floppy1_2, floppy1_3, floppy1_4, 0
floppy2:
        db floppy2_1, floppy2_2, floppy2_3, floppy2_4, 0
text:
        db C, e, c, i, 20h
        db n, apos_e, s, t, 20h
        db p, a, s_end, 20h
        db u, n, 20h
        db d, i, s, q, u, e_end, 20h
        db a_start, m, o, r, c_cedilla, a, b, l, e_end, period, 0

        db "github.com/oxidecomputer/cidata", 0
        times 510 - ($-$$) db 0
        dw 0xAA55

; Start of Conventional Memory after the boot sector.
end_boot_sector:
