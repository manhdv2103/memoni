use egui::Color32;

pub fn parse_color(input: &str) -> Option<Color32> {
    let s = input.trim().to_lowercase();

    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(args) = s.strip_prefix("rgb(").and_then(|x| x.strip_suffix(')')) {
        return parse_rgb(args, false);
    }
    if let Some(args) = s.strip_prefix("rgba(").and_then(|x| x.strip_suffix(')')) {
        return parse_rgb(args, true);
    }
    if let Some(args) = s.strip_prefix("hsl(").and_then(|x| x.strip_suffix(')')) {
        return parse_hsl(args, false);
    }
    if let Some(args) = s.strip_prefix("hsla(").and_then(|x| x.strip_suffix(')')) {
        return parse_hsl(args, true);
    }

    None
}

fn parse_hex(hex: &str) -> Option<Color32> {
    let expand = |c: u8| Some(((c as char).to_digit(16)? as u8) * 17);

    let hex_bytes = hex.as_bytes();
    let (r, g, b, a) = match hex.len() {
        3 => (
            expand(hex_bytes[0])?,
            expand(hex_bytes[1])?,
            expand(hex_bytes[2])?,
            255,
        ),
        4 => (
            expand(hex_bytes[0])?,
            expand(hex_bytes[1])?,
            expand(hex_bytes[2])?,
            expand(hex_bytes[3])?,
        ),
        6 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            255,
        ),
        8 => (
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            u8::from_str_radix(&hex[6..8], 16).ok()?,
        ),
        _ => return None,
    };

    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

fn parse_rgb(args: &str, has_alpha: bool) -> Option<Color32> {
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    if parts.len() != if has_alpha { 4 } else { 3 } {
        return None;
    }

    let r = parts[0].parse::<u8>().ok()?;
    let g = parts[1].parse::<u8>().ok()?;
    let b = parts[2].parse::<u8>().ok()?;
    let a = if has_alpha {
        let v = parts[3].parse::<f32>().ok()?.clamp(0.0, 1.0);
        (v * 255.0).round() as u8
    } else {
        255
    };

    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

fn parse_hsl(args: &str, has_alpha: bool) -> Option<Color32> {
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    if parts.len() != if has_alpha { 4 } else { 3 } {
        return None;
    }

    let h = parts[0].parse::<f32>().ok()? % 360.0;
    let s = parts[1].strip_suffix('%')?.parse::<f32>().ok()? / 100.0;
    let l = parts[2].strip_suffix('%')?.parse::<f32>().ok()? / 100.0;
    let a = if has_alpha {
        let v = parts[3].parse::<f32>().ok()?.clamp(0.0, 1.0);
        (v * 255.0).round() as u8
    } else {
        255
    };

    let (r, g, b) = hsl_to_rgb(h, s, l);
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = match h {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
    )
}
