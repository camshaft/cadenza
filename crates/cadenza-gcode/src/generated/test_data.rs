use crate::parse;
use insta::assert_debug_snapshot as s;

mod checksums {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "; GCode with checksums\n; Checksum is XOR of all bytes before the asterisk\nG28*18\nG1 X100 Y50*57\nM104 S200*99\nG90*21\n";
        let parse = parse(gcode);
        let cst = parse.syntax();
        s!(
            "checksums_cst",
            &cst,
            "; GCode with checksums\n; Checksum is XOR of all bytes before the asterisk\nG28*18\nG1 X100 Y50*57\nM104 S200*99\nG90*21\n"
        );
    }
    #[test]
    fn ast() {
        let gcode = "; GCode with checksums\n; Checksum is XOR of all bytes before the asterisk\nG28*18\nG1 X100 Y50*57\nM104 S200*99\nG90*21\n";
        let parse = parse(gcode);
        let root = parse.ast();
        let ast_debug = format!("{:?}", root);
        s!(
            "checksums_ast",
            ast_debug,
            "; GCode with checksums\n; Checksum is XOR of all bytes before the asterisk\nG28*18\nG1 X100 Y50*57\nM104 S200*99\nG90*21\n"
        );
    }
}
mod complex {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "; RepRap 3D Printer Test GCode\n; Demonstrates various GCode commands\n\n; ===== Initialization =====\nG28                    ; Home all axes\nG90                    ; Set absolute positioning\nG92 E0                 ; Reset extruder position\n\n; ===== Heating =====\nM104 S200              ; Set extruder temp to 200C (non-blocking)\nM140 S60               ; Set bed temp to 60C (non-blocking)\nM109 S200              ; Wait for extruder temp\nM190 S60               ; Wait for bed temp\n\n; ===== First Layer =====\nG1 Z0.2 F3000          ; Move to first layer height\nG1 X20 Y20 F3000       ; Move to start position\nM106 S255              ; Turn fan on full speed\n\n; ===== Drawing Rectangle =====\nG1 X100 Y20 E5 F1500   ; Draw line with extrusion\nG1 X100 Y100 E10       ; Continue line\nG1 X20 Y100 E15        ; Continue line\nG1 X20 Y20 E20         ; Complete rectangle\n\n; ===== Finishing =====\nG1 Z50 F3000           ; Move up\nM107                   ; Turn fan off\nM104 S0                ; Turn off extruder\nM140 S0                ; Turn off bed\nG28 X Y                ; Home X and Y\nM82                    ; Set absolute E coordinates\n";
        let parse = parse(gcode);
        let cst = parse.syntax();
        s!(
            "complex_cst",
            &cst,
            "; RepRap 3D Printer Test GCode\n; Demonstrates various GCode commands\n\n; ===== Initialization =====\nG28                    ; Home all axes\nG90                    ; Set absolute positioning\nG92 E0                 ; Reset extruder position\n\n; ===== Heating =====\nM104 S200              ; Set extruder temp to 200C (non-blocking)\nM140 S60               ; Set bed temp to 60C (non-blocking)\nM109 S200              ; Wait for extruder temp\nM190 S60               ; Wait for bed temp\n\n; ===== First Layer =====\nG1 Z0.2 F3000          ; Move to first layer height\nG1 X20 Y20 F3000       ; Move to start position\nM106 S255              ; Turn fan on full speed\n\n; ===== Drawing Rectangle =====\nG1 X100 Y20 E5 F1500   ; Draw line with extrusion\nG1 X100 Y100 E10       ; Continue line\nG1 X20 Y100 E15        ; Continue line\nG1 X20 Y20 E20         ; Complete rectangle\n\n; ===== Finishing =====\nG1 Z50 F3000           ; Move up\nM107                   ; Turn fan off\nM104 S0                ; Turn off extruder\nM140 S0                ; Turn off bed\nG28 X Y                ; Home X and Y\nM82                    ; Set absolute E coordinates\n"
        );
    }
    #[test]
    fn ast() {
        let gcode = "; RepRap 3D Printer Test GCode\n; Demonstrates various GCode commands\n\n; ===== Initialization =====\nG28                    ; Home all axes\nG90                    ; Set absolute positioning\nG92 E0                 ; Reset extruder position\n\n; ===== Heating =====\nM104 S200              ; Set extruder temp to 200C (non-blocking)\nM140 S60               ; Set bed temp to 60C (non-blocking)\nM109 S200              ; Wait for extruder temp\nM190 S60               ; Wait for bed temp\n\n; ===== First Layer =====\nG1 Z0.2 F3000          ; Move to first layer height\nG1 X20 Y20 F3000       ; Move to start position\nM106 S255              ; Turn fan on full speed\n\n; ===== Drawing Rectangle =====\nG1 X100 Y20 E5 F1500   ; Draw line with extrusion\nG1 X100 Y100 E10       ; Continue line\nG1 X20 Y100 E15        ; Continue line\nG1 X20 Y20 E20         ; Complete rectangle\n\n; ===== Finishing =====\nG1 Z50 F3000           ; Move up\nM107                   ; Turn fan off\nM104 S0                ; Turn off extruder\nM140 S0                ; Turn off bed\nG28 X Y                ; Home X and Y\nM82                    ; Set absolute E coordinates\n";
        let parse = parse(gcode);
        let root = parse.ast();
        let ast_debug = format!("{:?}", root);
        s!(
            "complex_ast",
            ast_debug,
            "; RepRap 3D Printer Test GCode\n; Demonstrates various GCode commands\n\n; ===== Initialization =====\nG28                    ; Home all axes\nG90                    ; Set absolute positioning\nG92 E0                 ; Reset extruder position\n\n; ===== Heating =====\nM104 S200              ; Set extruder temp to 200C (non-blocking)\nM140 S60               ; Set bed temp to 60C (non-blocking)\nM109 S200              ; Wait for extruder temp\nM190 S60               ; Wait for bed temp\n\n; ===== First Layer =====\nG1 Z0.2 F3000          ; Move to first layer height\nG1 X20 Y20 F3000       ; Move to start position\nM106 S255              ; Turn fan on full speed\n\n; ===== Drawing Rectangle =====\nG1 X100 Y20 E5 F1500   ; Draw line with extrusion\nG1 X100 Y100 E10       ; Continue line\nG1 X20 Y100 E15        ; Continue line\nG1 X20 Y20 E20         ; Complete rectangle\n\n; ===== Finishing =====\nG1 Z50 F3000           ; Move up\nM107                   ; Turn fan off\nM104 S0                ; Turn off extruder\nM140 S0                ; Turn off bed\nG28 X Y                ; Home X and Y\nM82                    ; Set absolute E coordinates\n"
        );
    }
}
mod klipper {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "; Klipper macro format with named parameters\nSET_PIN PIN=my_led VALUE=1\nSET_HEATER_TEMPERATURE HEATER=extruder TARGET=200\nSET_VELOCITY_LIMIT VELOCITY=100 ACCEL=3000\nG28 X Y\nTEMPERATURE_WAIT SENSOR=extruder MINIMUM=195 MAXIMUM=205\n";
        let parse = parse(gcode);
        let cst = parse.syntax();
        s!(
            "klipper_cst",
            &cst,
            "; Klipper macro format with named parameters\nSET_PIN PIN=my_led VALUE=1\nSET_HEATER_TEMPERATURE HEATER=extruder TARGET=200\nSET_VELOCITY_LIMIT VELOCITY=100 ACCEL=3000\nG28 X Y\nTEMPERATURE_WAIT SENSOR=extruder MINIMUM=195 MAXIMUM=205\n"
        );
    }
    #[test]
    fn ast() {
        let gcode = "; Klipper macro format with named parameters\nSET_PIN PIN=my_led VALUE=1\nSET_HEATER_TEMPERATURE HEATER=extruder TARGET=200\nSET_VELOCITY_LIMIT VELOCITY=100 ACCEL=3000\nG28 X Y\nTEMPERATURE_WAIT SENSOR=extruder MINIMUM=195 MAXIMUM=205\n";
        let parse = parse(gcode);
        let root = parse.ast();
        let ast_debug = format!("{:?}", root);
        s!(
            "klipper_ast",
            ast_debug,
            "; Klipper macro format with named parameters\nSET_PIN PIN=my_led VALUE=1\nSET_HEATER_TEMPERATURE HEATER=extruder TARGET=200\nSET_VELOCITY_LIMIT VELOCITY=100 ACCEL=3000\nG28 X Y\nTEMPERATURE_WAIT SENSOR=extruder MINIMUM=195 MAXIMUM=205\n"
        );
    }
}
mod simple {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "; Simple test GCode file\n; Home all axes\nG28\n\n; Set absolute positioning\nG90\n\n; Heat extruder\nM104 S200\n\n; Move to starting position\nG1 X10 Y10 Z0.2 F3000\n\n; Draw a simple square\nG1 X100 Y10 F1500\nG1 X100 Y100\nG1 X10 Y100\nG1 X10 Y10\n\n; Finish\nG28\nM104 S0\n";
        let parse = parse(gcode);
        let cst = parse.syntax();
        s!(
            "simple_cst",
            &cst,
            "; Simple test GCode file\n; Home all axes\nG28\n\n; Set absolute positioning\nG90\n\n; Heat extruder\nM104 S200\n\n; Move to starting position\nG1 X10 Y10 Z0.2 F3000\n\n; Draw a simple square\nG1 X100 Y10 F1500\nG1 X100 Y100\nG1 X10 Y100\nG1 X10 Y10\n\n; Finish\nG28\nM104 S0\n"
        );
    }
    #[test]
    fn ast() {
        let gcode = "; Simple test GCode file\n; Home all axes\nG28\n\n; Set absolute positioning\nG90\n\n; Heat extruder\nM104 S200\n\n; Move to starting position\nG1 X10 Y10 Z0.2 F3000\n\n; Draw a simple square\nG1 X100 Y10 F1500\nG1 X100 Y100\nG1 X10 Y100\nG1 X10 Y10\n\n; Finish\nG28\nM104 S0\n";
        let parse = parse(gcode);
        let root = parse.ast();
        let ast_debug = format!("{:?}", root);
        s!(
            "simple_ast",
            ast_debug,
            "; Simple test GCode file\n; Home all axes\nG28\n\n; Set absolute positioning\nG90\n\n; Heat extruder\nM104 S200\n\n; Move to starting position\nG1 X10 Y10 Z0.2 F3000\n\n; Draw a simple square\nG1 X100 Y10 F1500\nG1 X100 Y100\nG1 X10 Y100\nG1 X10 Y10\n\n; Finish\nG28\nM104 S0\n"
        );
    }
}
