; Simple test GCode file
; Home all axes
G28

; Set absolute positioning
G90

; Heat extruder
M104 S200

; Move to starting position
G1 X10 Y10 Z0.2 F3000

; Draw a simple square
G1 X100 Y10 F1500
G1 X100 Y100
G1 X10 Y100
G1 X10 Y10

; Finish
G28
M104 S0
