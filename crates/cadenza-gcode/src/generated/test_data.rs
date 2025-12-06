use crate::{parse, testing::verify_cst_coverage};
use insta::assert_debug_snapshot as s;

mod checksums {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "; GCode with checksums\n; Checksum is XOR of all bytes before the asterisk\nG28*18\nG1 X100 Y50*57\nM104 S200*99\nG90*21\n";
        let parse = parse(gcode);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

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
        s!(
            "checksums_ast",
            root,
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

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

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
        s!(
            "complex_ast",
            root,
            "; RepRap 3D Printer Test GCode\n; Demonstrates various GCode commands\n\n; ===== Initialization =====\nG28                    ; Home all axes\nG90                    ; Set absolute positioning\nG92 E0                 ; Reset extruder position\n\n; ===== Heating =====\nM104 S200              ; Set extruder temp to 200C (non-blocking)\nM140 S60               ; Set bed temp to 60C (non-blocking)\nM109 S200              ; Wait for extruder temp\nM190 S60               ; Wait for bed temp\n\n; ===== First Layer =====\nG1 Z0.2 F3000          ; Move to first layer height\nG1 X20 Y20 F3000       ; Move to start position\nM106 S255              ; Turn fan on full speed\n\n; ===== Drawing Rectangle =====\nG1 X100 Y20 E5 F1500   ; Draw line with extrusion\nG1 X100 Y100 E10       ; Continue line\nG1 X20 Y100 E15        ; Continue line\nG1 X20 Y20 E20         ; Complete rectangle\n\n; ===== Finishing =====\nG1 Z50 F3000           ; Move up\nM107                   ; Turn fan off\nM104 S0                ; Turn off extruder\nM140 S0                ; Turn off bed\nG28 X Y                ; Home X and Y\nM82                    ; Set absolute E coordinates\n"
        );
    }
}
mod edge_cases {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "; Edge cases test\n(Unclosed comment\nN100 G28\nN G28\n(Normal) G1 X10 (Another comment) Y20 ; Semicolon too\n%%\n";
        let parse = parse(gcode);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

        s!(
            "edge_cases_cst",
            &cst,
            "; Edge cases test\n(Unclosed comment\nN100 G28\nN G28\n(Normal) G1 X10 (Another comment) Y20 ; Semicolon too\n%%\n"
        );
    }
    #[test]
    fn ast() {
        let gcode = "; Edge cases test\n(Unclosed comment\nN100 G28\nN G28\n(Normal) G1 X10 (Another comment) Y20 ; Semicolon too\n%%\n";
        let parse = parse(gcode);
        let root = parse.ast();
        s!(
            "edge_cases_ast",
            root,
            "; Edge cases test\n(Unclosed comment\nN100 G28\nN G28\n(Normal) G1 X10 (Another comment) Y20 ; Semicolon too\n%%\n"
        );
    }
}
mod invalid_checksum {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "; Invalid checksum test\nG28*99\nG1 X100*11\n";
        let parse = parse(gcode);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

        s!(
            "invalid_checksum_cst",
            &cst,
            "; Invalid checksum test\nG28*99\nG1 X100*11\n"
        );
    }
    #[test]
    fn ast() {
        let gcode = "; Invalid checksum test\nG28*99\nG1 X100*11\n";
        let parse = parse(gcode);
        let root = parse.ast();
        s!(
            "invalid_checksum_ast",
            root,
            "; Invalid checksum test\nG28*99\nG1 X100*11\n"
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

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

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
        s!(
            "klipper_ast",
            root,
            "; Klipper macro format with named parameters\nSET_PIN PIN=my_led VALUE=1\nSET_HEATER_TEMPERATURE HEATER=extruder TARGET=200\nSET_VELOCITY_LIMIT VELOCITY=100 ACCEL=3000\nG28 X Y\nTEMPERATURE_WAIT SENSOR=extruder MINIMUM=195 MAXIMUM=205\n"
        );
    }
}
mod line_numbers {
    use super::*;
    #[test]
    fn cst() {
        let gcode =
            "; GCode with line numbers\nN10 G28\nN20 G1 X100 Y50 F3000\nN30 M104 S200\nN40 G90\n";
        let parse = parse(gcode);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

        s!(
            "line_numbers_cst",
            &cst,
            "; GCode with line numbers\nN10 G28\nN20 G1 X100 Y50 F3000\nN30 M104 S200\nN40 G90\n"
        );
    }
    #[test]
    fn ast() {
        let gcode =
            "; GCode with line numbers\nN10 G28\nN20 G1 X100 Y50 F3000\nN30 M104 S200\nN40 G90\n";
        let parse = parse(gcode);
        let root = parse.ast();
        s!(
            "line_numbers_ast",
            root,
            "; GCode with line numbers\nN10 G28\nN20 G1 X100 Y50 F3000\nN30 M104 S200\nN40 G90\n"
        );
    }
}
mod mixed_features {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "%\n(3D Print Program)\n; Mixed features test\nN10 G28 (Home)\nN20 G1 X100 Y50 F3000 ; Move\nN30 M104 S200 (Heat extruder)\n%\n";
        let parse = parse(gcode);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

        s!(
            "mixed_features_cst",
            &cst,
            "%\n(3D Print Program)\n; Mixed features test\nN10 G28 (Home)\nN20 G1 X100 Y50 F3000 ; Move\nN30 M104 S200 (Heat extruder)\n%\n"
        );
    }
    #[test]
    fn ast() {
        let gcode = "%\n(3D Print Program)\n; Mixed features test\nN10 G28 (Home)\nN20 G1 X100 Y50 F3000 ; Move\nN30 M104 S200 (Heat extruder)\n%\n";
        let parse = parse(gcode);
        let root = parse.ast();
        s!(
            "mixed_features_ast",
            root,
            "%\n(3D Print Program)\n; Mixed features test\nN10 G28 (Home)\nN20 G1 X100 Y50 F3000 ; Move\nN30 M104 S200 (Heat extruder)\n%\n"
        );
    }
}
mod parentheses_comments {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "; GCode with parentheses-style comments\n(Program start)\nG28 (Home all axes)\nG1 X100 Y50 (Move to position)\n(Set temperature)\nM104 S200\n";
        let parse = parse(gcode);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

        s!(
            "parentheses_comments_cst",
            &cst,
            "; GCode with parentheses-style comments\n(Program start)\nG28 (Home all axes)\nG1 X100 Y50 (Move to position)\n(Set temperature)\nM104 S200\n"
        );
    }
    #[test]
    fn ast() {
        let gcode = "; GCode with parentheses-style comments\n(Program start)\nG28 (Home all axes)\nG1 X100 Y50 (Move to position)\n(Set temperature)\nM104 S200\n";
        let parse = parse(gcode);
        let root = parse.ast();
        s!(
            "parentheses_comments_ast",
            root,
            "; GCode with parentheses-style comments\n(Program start)\nG28 (Home all axes)\nG1 X100 Y50 (Move to position)\n(Set temperature)\nM104 S200\n"
        );
    }
}
mod percent_delimiters {
    use super::*;
    #[test]
    fn cst() {
        let gcode = "%\n; Program body\nG28\nG1 X100 Y50\nM104 S200\n%\n";
        let parse = parse(gcode);
        let cst = parse.syntax();

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

        s!(
            "percent_delimiters_cst",
            &cst,
            "%\n; Program body\nG28\nG1 X100 Y50\nM104 S200\n%\n"
        );
    }
    #[test]
    fn ast() {
        let gcode = "%\n; Program body\nG28\nG1 X100 Y50\nM104 S200\n%\n";
        let parse = parse(gcode);
        let root = parse.ast();
        s!(
            "percent_delimiters_ast",
            root,
            "%\n; Program body\nG28\nG1 X100 Y50\nM104 S200\n%\n"
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

        // Verify CST span coverage and token text accuracy
        verify_cst_coverage(gcode);

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
        s!(
            "simple_ast",
            root,
            "; Simple test GCode file\n; Home all axes\nG28\n\n; Set absolute positioning\nG90\n\n; Heat extruder\nM104 S200\n\n; Move to starting position\nG1 X10 Y10 Z0.2 F3000\n\n; Draw a simple square\nG1 X100 Y10 F1500\nG1 X100 Y100\nG1 X10 Y100\nG1 X10 Y10\n\n; Finish\nG28\nM104 S0\n"
        );
    }
}
