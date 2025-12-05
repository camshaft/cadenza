use crate::parse;
use insta::assert_snapshot as s;

mod complex {
    use super::*;
    #[test]
    fn parse_ast() {
        let gcode = "; RepRap 3D Printer Test GCode\n; Demonstrates various GCode commands\n\n; ===== Initialization =====\nG28                    ; Home all axes\nG90                    ; Set absolute positioning\nG92 E0                 ; Reset extruder position\n\n; ===== Heating =====\nM104 S200              ; Set extruder temp to 200C (non-blocking)\nM140 S60               ; Set bed temp to 60C (non-blocking)\nM109 S200              ; Wait for extruder temp\nM190 S60               ; Wait for bed temp\n\n; ===== First Layer =====\nG1 Z0.2 F3000          ; Move to first layer height\nG1 X20 Y20 F3000       ; Move to start position\nM106 S255              ; Turn fan on full speed\n\n; ===== Drawing Rectangle =====\nG1 X100 Y20 E5 F1500   ; Draw line with extrusion\nG1 X100 Y100 E10       ; Continue line\nG1 X20 Y100 E15        ; Continue line\nG1 X20 Y20 E20         ; Complete rectangle\n\n; ===== Finishing =====\nG1 Z50 F3000           ; Move up\nM107                   ; Turn fan off\nM104 S0                ; Turn off extruder\nM140 S0                ; Turn off bed\nG28 X Y                ; Home X and Y\nM82                    ; Set absolute E coordinates\n";
        let parse = parse(gcode);
        let root = parse.ast();
        let ast_debug = format!("{:?}", root);
        s!("complex_parse_ast", ast_debug);
    }
}
mod simple {
    use super::*;
    #[test]
    fn parse_ast() {
        let gcode = "; Simple test GCode file\n; Home all axes\nG28\n\n; Set absolute positioning\nG90\n\n; Heat extruder\nM104 S200\n\n; Move to starting position\nG1 X10 Y10 Z0.2 F3000\n\n; Draw a simple square\nG1 X100 Y10 F1500\nG1 X100 Y100\nG1 X10 Y100\nG1 X10 Y10\n\n; Finish\nG28\nM104 S0\n";
        let parse = parse(gcode);
        let root = parse.ast();
        let ast_debug = format!("{:?}", root);
        s!("simple_parse_ast", ast_debug);
    }
}
