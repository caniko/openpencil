#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthCtx {
    pub font_size: f64,
    pub root_font_size: f64,
    pub viewport_w: f64,
    pub viewport_h: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CssLength {
    Px(f64),
    Percent(f64),
}

pub fn parse_length(value: &str, ctx: &LengthCtx) -> Option<CssLength> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(number) = value.strip_suffix("rem") {
        return Some(CssLength::Px(
            number.parse::<f64>().ok()? * ctx.root_font_size,
        ));
    }
    if let Some(number) = value.strip_suffix("px") {
        return Some(CssLength::Px(number.parse().ok()?));
    }
    if let Some(number) = value.strip_suffix("em") {
        return Some(CssLength::Px(number.parse::<f64>().ok()? * ctx.font_size));
    }
    if let Some(number) = value.strip_suffix('%') {
        return Some(CssLength::Percent(number.parse().ok()?));
    }
    if let Some(number) = value.strip_suffix("vw") {
        return Some(CssLength::Px(
            number.parse::<f64>().ok()? * ctx.viewport_w / 100.0,
        ));
    }
    if let Some(number) = value.strip_suffix("vh") {
        return Some(CssLength::Px(
            number.parse::<f64>().ok()? * ctx.viewport_h / 100.0,
        ));
    }
    if let Some(number) = value.strip_suffix("pt") {
        return Some(CssLength::Px(number.parse::<f64>().ok()? * 4.0 / 3.0));
    }
    let number = value.parse::<f64>().ok()?;
    (number == 0.0).then_some(CssLength::Px(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units() {
        let ctx = LengthCtx {
            font_size: 20.0,
            root_font_size: 16.0,
            viewport_w: 1440.0,
            viewport_h: 900.0,
        };
        assert!(matches!(
            parse_length("24px", &ctx),
            Some(CssLength::Px(v)) if v == 24.0
        ));
        assert!(matches!(
            parse_length("1.5em", &ctx),
            Some(CssLength::Px(v)) if v == 30.0
        ));
        assert!(matches!(
            parse_length("2rem", &ctx),
            Some(CssLength::Px(v)) if v == 32.0
        ));
        assert!(matches!(
            parse_length("50%", &ctx),
            Some(CssLength::Percent(v)) if v == 50.0
        ));
        assert!(matches!(
            parse_length("10vw", &ctx),
            Some(CssLength::Px(v)) if v == 144.0
        ));
        assert!(matches!(
            parse_length("0", &ctx),
            Some(CssLength::Px(v)) if v == 0.0
        ));
        assert!(parse_length("auto", &ctx).is_none());
    }
}
