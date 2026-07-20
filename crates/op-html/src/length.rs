use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthCtx {
    pub font_size: f64,
    pub root_font_size: f64,
    pub viewport_w: f64,
    pub viewport_h: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CssLength {
    Px(f64),
    Percent(f64),
    Mixed {
        px: f64,
        percent: f64,
    },
    Sum(Box<CssLength>, Box<CssLength>),
    Scale(Box<CssLength>, f64),
    Min(Vec<CssLength>),
    Max(Vec<CssLength>),
    Clamp {
        minimum: Box<CssLength>,
        preferred: Box<CssLength>,
        maximum: Box<CssLength>,
    },
}

impl CssLength {
    pub fn resolve(&self, reference: f64) -> f64 {
        match self {
            Self::Px(px) => *px,
            Self::Percent(percent) => reference * percent / 100.0,
            Self::Mixed { px, percent } => px + reference * percent / 100.0,
            Self::Sum(left, right) => left.resolve(reference) + right.resolve(reference),
            Self::Scale(length, factor) => length.resolve(reference) * factor,
            Self::Min(values) => values
                .iter()
                .map(|value| value.resolve(reference))
                .reduce(f64::min)
                .unwrap_or(0.0),
            Self::Max(values) => values
                .iter()
                .map(|value| value.resolve(reference))
                .reduce(f64::max)
                .unwrap_or(0.0),
            Self::Clamp {
                minimum,
                preferred,
                maximum,
            } => minimum
                .resolve(reference)
                .max(preferred.resolve(reference).min(maximum.resolve(reference))),
        }
    }

    pub(crate) fn depends_on_reference(&self) -> bool {
        match self {
            Self::Px(_) => false,
            Self::Percent(_) | Self::Mixed { .. } => true,
            Self::Sum(left, right) => left.depends_on_reference() || right.depends_on_reference(),
            Self::Scale(length, _) => length.depends_on_reference(),
            Self::Min(values) | Self::Max(values) => values.iter().any(Self::depends_on_reference),
            Self::Clamp {
                minimum,
                preferred,
                maximum,
            } => {
                minimum.depends_on_reference()
                    || preferred.depends_on_reference()
                    || maximum.depends_on_reference()
            }
        }
    }

    fn affine(&self) -> Option<AffineLength> {
        match self {
            Self::Px(px) => Some(AffineLength {
                px: *px,
                percent: 0.0,
            }),
            Self::Percent(percent) => Some(AffineLength {
                px: 0.0,
                percent: *percent,
            }),
            Self::Mixed { px, percent } => Some(AffineLength {
                px: *px,
                percent: *percent,
            }),
            Self::Sum(left, right) => left.affine()?.add(right.affine()?),
            Self::Scale(length, factor) => length.affine()?.scale(*factor),
            Self::Min(_) | Self::Max(_) | Self::Clamp { .. } => None,
        }
    }

    fn add(self, other: Self) -> Option<Self> {
        match (self.affine(), other.affine()) {
            (Some(left), Some(right)) => Some(left.add(right)?.into_css_length()),
            _ => Some(Self::Sum(Box::new(self), Box::new(other))),
        }
    }

    fn scale(self, factor: f64) -> Option<Self> {
        finite(factor)?;
        if let Some(length) = self.affine() {
            return Some(length.scale(factor)?.into_css_length());
        }
        if factor == 0.0 {
            Some(Self::Px(0.0))
        } else {
            Some(Self::Scale(Box::new(self), factor))
        }
    }

    fn extrema(values: Vec<Self>, maximum: bool) -> Option<Self> {
        let mut selected = values.first()?.clone();
        for candidate in values.iter().skip(1) {
            let Some(ordering) = selected.compare_independent(candidate) else {
                return Some(if maximum {
                    Self::Max(values)
                } else {
                    Self::Min(values)
                });
            };
            if (maximum && ordering == Ordering::Less)
                || (!maximum && ordering == Ordering::Greater)
            {
                selected = candidate.clone();
            }
        }
        Some(selected)
    }

    fn clamp(minimum: Self, preferred: Self, maximum: Self) -> Self {
        let capped = preferred.compare_independent(&maximum).map(|ordering| {
            if ordering == Ordering::Greater {
                maximum.clone()
            } else {
                preferred.clone()
            }
        });
        if let Some(capped) = capped {
            if let Some(ordering) = minimum.compare_independent(&capped) {
                return if ordering == Ordering::Greater {
                    minimum
                } else {
                    capped
                };
            }
        }
        Self::Clamp {
            minimum: Box::new(minimum),
            preferred: Box::new(preferred),
            maximum: Box::new(maximum),
        }
    }

    fn compare_independent(&self, other: &Self) -> Option<Ordering> {
        self.affine()?.compare(other.affine()?)
    }
}

pub fn parse_length(value: &str, ctx: &LengthCtx) -> Option<CssLength> {
    let input = value.trim().to_ascii_lowercase();
    let mut parser = Parser {
        input: &input,
        pos: 0,
        ctx,
    };
    let result = parser.parse_sum(0)?;
    parser.skip_whitespace();
    if parser.pos != parser.input.len() {
        return None;
    }
    match result {
        CalcValue::Number(0.0) => Some(CssLength::Px(0.0)),
        CalcValue::Number(_) => None,
        CalcValue::Length(length) => Some(length),
    }
}

#[derive(Clone, Copy, Debug)]
struct AffineLength {
    px: f64,
    percent: f64,
}

impl AffineLength {
    fn into_css_length(self) -> CssLength {
        if self.percent == 0.0 {
            CssLength::Px(self.px)
        } else if self.px == 0.0 {
            CssLength::Percent(self.percent)
        } else {
            CssLength::Mixed {
                px: self.px,
                percent: self.percent,
            }
        }
    }

    fn scale(self, factor: f64) -> Option<Self> {
        let result = Self {
            px: self.px * factor,
            percent: self.percent * factor,
        };
        (result.px.is_finite() && result.percent.is_finite()).then_some(result)
    }

    fn add(self, other: Self) -> Option<Self> {
        Some(Self {
            px: finite(self.px + other.px)?,
            percent: finite(self.percent + other.percent)?,
        })
    }

    fn compare(self, other: Self) -> Option<Ordering> {
        let px = self.px - other.px;
        let percent = self.percent - other.percent;
        if px == 0.0 {
            return percent.partial_cmp(&0.0);
        }
        if percent == 0.0 || px.signum() == percent.signum() {
            return px.partial_cmp(&0.0);
        }
        // Opposite signs cross at some non-negative percentage reference.
        None
    }
}

#[derive(Clone, Debug)]
enum CalcValue {
    Number(f64),
    Length(CssLength),
}

impl CalcValue {
    fn negate(self) -> Option<Self> {
        self.multiply(Self::Number(-1.0))
    }

    fn add(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Number(left), Self::Number(right)) => finite_number(left + right),
            (Self::Length(left), Self::Length(right)) => Some(Self::Length(left.add(right)?)),
            (Self::Number(0.0), length @ Self::Length(_))
            | (length @ Self::Length(_), Self::Number(0.0)) => Some(length),
            _ => None,
        }
    }

    fn subtract(self, other: Self) -> Option<Self> {
        self.add(other.negate()?)
    }

    fn multiply(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Number(left), Self::Number(right)) => finite_number(left * right),
            (Self::Length(length), Self::Number(factor))
            | (Self::Number(factor), Self::Length(length)) => {
                Some(Self::Length(length.scale(factor)?))
            }
            _ => None,
        }
    }

    fn divide(self, other: Self) -> Option<Self> {
        match other {
            Self::Number(number) if number != 0.0 => self.multiply(Self::Number(1.0 / number)),
            _ => None,
        }
    }

    fn compare_independent(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Number(left), Self::Number(right)) => left.partial_cmp(right),
            (Self::Length(left), Self::Length(right)) => left.compare_independent(right),
            _ => None,
        }
    }

    fn extrema(values: Vec<Self>, maximum: bool) -> Option<Self> {
        if values.iter().all(|value| matches!(value, Self::Number(_))) {
            let mut selected = values.first()?.clone();
            for candidate in values.iter().skip(1) {
                let ordering = selected.compare_independent(candidate)?;
                if (maximum && ordering == Ordering::Less)
                    || (!maximum && ordering == Ordering::Greater)
                {
                    selected = candidate.clone();
                }
            }
            return Some(selected);
        }
        let lengths = values
            .into_iter()
            .map(|value| match value {
                Self::Length(length) => Some(length),
                Self::Number(_) => None,
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self::Length(CssLength::extrema(lengths, maximum)?))
    }

    fn clamp(minimum: Self, preferred: Self, maximum: Self) -> Option<Self> {
        match (minimum, preferred, maximum) {
            (Self::Number(minimum), Self::Number(preferred), Self::Number(maximum)) => {
                finite_number(minimum.max(preferred.min(maximum)))
            }
            (Self::Length(minimum), Self::Length(preferred), Self::Length(maximum)) => {
                Some(Self::Length(CssLength::clamp(minimum, preferred, maximum)))
            }
            _ => None,
        }
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn finite_number(value: f64) -> Option<CalcValue> {
    Some(CalcValue::Number(finite(value)?))
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
    ctx: &'a LengthCtx,
}

const MAX_PARSE_DEPTH: usize = 64;

impl Parser<'_> {
    fn parse_sum(&mut self, depth: usize) -> Option<CalcValue> {
        (depth <= MAX_PARSE_DEPTH).then_some(())?;
        let mut value = self.parse_product(depth)?;
        loop {
            self.skip_whitespace();
            if self.consume('+') {
                value = value.add(self.parse_product(depth)?)?;
            } else if self.consume('-') {
                value = value.subtract(self.parse_product(depth)?)?;
            } else {
                return Some(value);
            }
        }
    }

    fn parse_product(&mut self, depth: usize) -> Option<CalcValue> {
        let mut value = self.parse_unary(depth)?;
        loop {
            self.skip_whitespace();
            if self.consume('*') {
                value = value.multiply(self.parse_unary(depth)?)?;
            } else if self.consume('/') {
                value = value.divide(self.parse_unary(depth)?)?;
            } else {
                return Some(value);
            }
        }
    }

    fn parse_unary(&mut self, depth: usize) -> Option<CalcValue> {
        (depth <= MAX_PARSE_DEPTH).then_some(())?;
        self.skip_whitespace();
        if self.consume('+') {
            self.parse_unary(depth + 1)
        } else if self.consume('-') {
            self.parse_unary(depth + 1)?.negate()
        } else {
            self.parse_primary(depth)
        }
    }

    fn parse_primary(&mut self, depth: usize) -> Option<CalcValue> {
        self.skip_whitespace();
        if self.consume('(') {
            let result = self.parse_sum(depth + 1)?;
            self.skip_whitespace();
            return self.consume(')').then_some(result);
        }
        if self.peek()?.is_ascii_alphabetic() {
            return self.parse_function(depth);
        }
        self.parse_numeric_value()
    }

    fn parse_function(&mut self, depth: usize) -> Option<CalcValue> {
        let name = self.parse_identifier().to_owned();
        self.skip_whitespace();
        if !self.consume('(') {
            return None;
        }
        match name.as_str() {
            "calc" => {
                let value = self.parse_sum(depth + 1)?;
                self.skip_whitespace();
                self.consume(')').then_some(value)
            }
            "min" => self.parse_extrema(false, depth + 1),
            "max" => self.parse_extrema(true, depth + 1),
            "clamp" => self.parse_clamp(depth + 1),
            _ => None,
        }
    }

    fn parse_extrema(&mut self, maximum: bool, depth: usize) -> Option<CalcValue> {
        let mut values = vec![self.parse_sum(depth)?];
        loop {
            self.skip_whitespace();
            if self.consume(')') {
                return CalcValue::extrema(values, maximum);
            }
            if !self.consume(',') {
                return None;
            }
            values.push(self.parse_sum(depth)?);
        }
    }

    fn parse_clamp(&mut self, depth: usize) -> Option<CalcValue> {
        let minimum = self.parse_sum(depth)?;
        self.skip_whitespace();
        if !self.consume(',') {
            return None;
        }
        let preferred = self.parse_sum(depth)?;
        self.skip_whitespace();
        if !self.consume(',') {
            return None;
        }
        let maximum = self.parse_sum(depth)?;
        self.skip_whitespace();
        if !self.consume(')') {
            return None;
        }
        CalcValue::clamp(minimum, preferred, maximum)
    }

    fn parse_numeric_value(&mut self) -> Option<CalcValue> {
        let number = self.parse_number()?;
        let unit = if self.consume('%') {
            "%"
        } else {
            self.parse_identifier()
        };
        let length = match unit {
            "" => return finite_number(number),
            "px" => number,
            "rem" => number * self.ctx.root_font_size,
            "em" => number * self.ctx.font_size,
            "vw" | "svw" | "lvw" | "dvw" | "vi" | "svi" | "lvi" | "dvi" => {
                number * self.ctx.viewport_w / 100.0
            }
            "vh" | "svh" | "lvh" | "dvh" | "vb" | "svb" | "lvb" | "dvb" => {
                number * self.ctx.viewport_h / 100.0
            }
            "vmin" => number * self.ctx.viewport_w.min(self.ctx.viewport_h) / 100.0,
            "vmax" => number * self.ctx.viewport_w.max(self.ctx.viewport_h) / 100.0,
            "pt" => number * 4.0 / 3.0,
            "pc" => number * 16.0,
            "in" => number * 96.0,
            "cm" => number * 96.0 / 2.54,
            "mm" => number * 96.0 / 25.4,
            "q" => number * 96.0 / 101.6,
            "ch" | "ex" | "cap" | "ic" => number * self.ctx.font_size / 2.0,
            "rch" | "rex" | "rcap" | "ric" => number * self.ctx.root_font_size / 2.0,
            "lh" => number * self.ctx.font_size * 1.2,
            "rlh" => number * self.ctx.root_font_size * 1.2,
            "%" => return Some(CalcValue::Length(CssLength::Percent(finite(number)?))),
            _ => return None,
        };
        Some(CalcValue::Length(CssLength::Px(finite(length)?)))
    }

    fn parse_number(&mut self) -> Option<f64> {
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let mut digits = 0;
        while self.pos < bytes.len() && bytes[self.pos].is_ascii_digit() {
            self.pos += 1;
            digits += 1;
        }
        if self.pos < bytes.len() && bytes[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < bytes.len() && bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            self.pos = start;
            return None;
        }
        if self.pos < bytes.len() && matches!(bytes[self.pos], b'e' | b'E') {
            let exponent_start = self.pos;
            self.pos += 1;
            if self.pos < bytes.len() && matches!(bytes[self.pos], b'+' | b'-') {
                self.pos += 1;
            }
            let exponent_digits = self.pos;
            while self.pos < bytes.len() && bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            if self.pos == exponent_digits {
                self.pos = exponent_start;
            }
        }
        finite(self.input[start..self.pos].parse().ok()?)
    }

    fn parse_identifier(&mut self) -> &'_ str {
        let start = self.pos;
        while self.pos < self.input.len()
            && (self.input.as_bytes()[self.pos].is_ascii_alphabetic()
                || self.input.as_bytes()[self.pos] == b'-')
        {
            self.pos += 1;
        }
        &self.input[start..self.pos]
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.input[self.pos..].starts_with(expected) {
            self.pos += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> LengthCtx {
        LengthCtx {
            font_size: 20.0,
            root_font_size: 16.0,
            viewport_w: 1440.0,
            viewport_h: 900.0,
        }
    }

    fn px(value: &str) -> f64 {
        match parse_length(value, &context()).unwrap() {
            CssLength::Px(value) => value,
            other => panic!("expected px, got {other:?}"),
        }
    }

    #[test]
    fn parses_relative_and_viewport_units() {
        assert_eq!(px("24px"), 24.0);
        assert_eq!(px("1.5em"), 30.0);
        assert_eq!(px("2rem"), 32.0);
        assert_eq!(px("10vw"), 144.0);
        assert_eq!(px("10vh"), 90.0);
        assert_eq!(px("10vmin"), 90.0);
        assert_eq!(px("10vmax"), 144.0);
        assert_eq!(px("10dvw"), 144.0);
        assert_eq!(px("10svh"), 90.0);
        assert_eq!(px("2ch"), 20.0);
        assert_eq!(px("2ex"), 20.0);
        assert_eq!(px("2lh"), 48.0);
        assert!((px("2rlh") - 38.4).abs() < 1e-9);
    }

    #[test]
    fn parses_absolute_units() {
        assert!((px("72pt") - 96.0).abs() < 1e-9);
        assert_eq!(px("6pc"), 96.0);
        assert_eq!(px("1in"), 96.0);
        assert!((px("2.54cm") - 96.0).abs() < 1e-9);
        assert!((px("25.4mm") - 96.0).abs() < 1e-9);
        assert!((px("101.6q") - 96.0).abs() < 1e-9);
    }

    #[test]
    fn keeps_percentages_and_mixed_calculations() {
        assert_eq!(
            parse_length("50%", &context()),
            Some(CssLength::Percent(50.0))
        );
        let length = parse_length("calc(100% - 2rem)", &context()).unwrap();
        assert_eq!(
            length,
            CssLength::Mixed {
                px: -32.0,
                percent: 100.0
            }
        );
        assert_eq!(length.resolve(500.0), 468.0);
        assert_eq!(px("calc((10vw + 2rem) / 2)"), 88.0);
        assert_eq!(px("calc(3 * 2em)"), 120.0);
    }

    #[test]
    fn parses_min_max_and_clamp() {
        assert_eq!(px("min(1in, 10pc)"), 96.0);
        assert_eq!(
            parse_length("max(10%, 20%)", &context()),
            Some(CssLength::Percent(20.0))
        );
        assert_eq!(px("clamp(1rem, 5vw, 10rem)"), 72.0);
        assert_eq!(
            parse_length("min(calc(100% + 10px), calc(100% + 20px))", &context()),
            Some(CssLength::Mixed {
                px: 10.0,
                percent: 100.0
            })
        );
        let wrapped = parse_length("min(1180px, calc(100% - 40px))", &context()).unwrap();
        assert_eq!(wrapped.resolve(800.0), 760.0);
        assert_eq!(wrapped.resolve(1440.0), 1180.0);

        let responsive = parse_length("min(100%, 1200px)", &context()).unwrap();
        assert_eq!(responsive.resolve(800.0), 800.0);
        assert_eq!(responsive.resolve(1440.0), 1200.0);

        let clamped = parse_length("clamp(300px, 50%, 700px)", &context()).unwrap();
        assert_eq!(clamped.resolve(400.0), 300.0);
        assert_eq!(clamped.resolve(1000.0), 500.0);
        assert_eq!(clamped.resolve(2000.0), 700.0);

        let calculated = parse_length("calc(min(100%, 1200px) - 20px)", &context()).unwrap();
        assert_eq!(calculated.resolve(800.0), 780.0);
        assert_eq!(calculated.resolve(1440.0), 1180.0);
    }

    #[test]
    fn rejects_invalid_dimensions_and_math() {
        assert_eq!(parse_length("0", &context()), Some(CssLength::Px(0.0)));
        assert!(parse_length("12", &context()).is_none());
        assert!(parse_length("auto", &context()).is_none());
        assert!(parse_length("calc(1px * 2px)", &context()).is_none());
        assert!(parse_length("calc(1px / 0)", &context()).is_none());
        assert!(parse_length("calc(1px +)", &context()).is_none());
    }

    #[test]
    fn limits_recursive_parse_depth() {
        let accepted = format!(
            "{}1px{}",
            "(".repeat(MAX_PARSE_DEPTH),
            ")".repeat(MAX_PARSE_DEPTH)
        );
        assert!(parse_length(&accepted, &context()).is_some());
        let rejected = format!(
            "{}1px{}",
            "(".repeat(MAX_PARSE_DEPTH + 1),
            ")".repeat(MAX_PARSE_DEPTH + 1)
        );
        assert!(parse_length(&rejected, &context()).is_none());
    }
}
