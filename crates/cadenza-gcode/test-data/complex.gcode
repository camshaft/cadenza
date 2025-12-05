; RepRap 3D Printer Test GCode
; Demonstrates various GCode commands

; ===== Initialization =====
G28                    ; Home all axes
G90                    ; Set absolute positioning
G92 E0                 ; Reset extruder position

; ===== Heating =====
M104 S200              ; Set extruder temp to 200C (non-blocking)
M140 S60               ; Set bed temp to 60C (non-blocking)
M109 S200              ; Wait for extruder temp
M190 S60               ; Wait for bed temp

; ===== First Layer =====
G1 Z0.2 F3000          ; Move to first layer height
G1 X20 Y20 F3000       ; Move to start position
M106 S255              ; Turn fan on full speed

; ===== Drawing Rectangle =====
G1 X100 Y20 E5 F1500   ; Draw line with extrusion
G1 X100 Y100 E10       ; Continue line
G1 X20 Y100 E15        ; Continue line
G1 X20 Y20 E20         ; Complete rectangle

; ===== Finishing =====
G1 Z50 F3000           ; Move up
M107                   ; Turn fan off
M104 S0                ; Turn off extruder
M140 S0                ; Turn off bed
G28 X Y                ; Home X and Y
M82                    ; Set absolute E coordinates
