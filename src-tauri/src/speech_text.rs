//! Numbers as a narrator says them. Chatterbox reads words reliably but loops or invents speech
//! on digit strings ("0.5875", "23,500", "10000 kg"), and Whisper aligns better against words.
//! This is the only place digits, units, and percentages become words; the desktop keeps digits in
//! its passage text so captions and highlights still match the visible reply.

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
const SCALES: [&str; 5] = ["", "thousand", "million", "billion", "trillion"];

/// Written without separators, a run this long is an identifier or code; it is read digit by
/// digit rather than as a quantity.
const LONGEST_SPOKEN_QUANTITY: usize = 7;

/// Unit abbreviations read after a number, with their singular and plural names. Only
/// abbreviations that are not also ordinary words are listed, so "5 in" or "5 A" stay as written.
const UNITS: &[(&str, &str, &str)] = &[
    ("nm", "nanometer", "nanometers"),
    ("µm", "micrometer", "micrometers"),
    ("mm", "millimeter", "millimeters"),
    ("cm", "centimeter", "centimeters"),
    ("m", "meter", "meters"),
    ("km", "kilometer", "kilometers"),
    ("mg", "milligram", "milligrams"),
    ("g", "gram", "grams"),
    ("kg", "kilogram", "kilograms"),
    ("ms", "millisecond", "milliseconds"),
    ("s", "second", "seconds"),
    ("sec", "second", "seconds"),
    ("min", "minute", "minutes"),
    ("h", "hour", "hours"),
    ("hr", "hour", "hours"),
    ("hrs", "hour", "hours"),
    ("J", "joule", "joules"),
    ("kJ", "kilojoule", "kilojoules"),
    ("MJ", "megajoule", "megajoules"),
    ("GJ", "gigajoule", "gigajoules"),
    ("W", "watt", "watts"),
    ("kW", "kilowatt", "kilowatts"),
    ("MW", "megawatt", "megawatts"),
    ("GW", "gigawatt", "gigawatts"),
    ("TW", "terawatt", "terawatts"),
    ("Wh", "watt hour", "watt hours"),
    ("kWh", "kilowatt hour", "kilowatt hours"),
    ("MWh", "megawatt hour", "megawatt hours"),
    ("V", "volt", "volts"),
    ("mV", "millivolt", "millivolts"),
    ("kV", "kilovolt", "kilovolts"),
    ("Hz", "hertz", "hertz"),
    ("kHz", "kilohertz", "kilohertz"),
    ("MHz", "megahertz", "megahertz"),
    ("GHz", "gigahertz", "gigahertz"),
    ("mL", "milliliter", "milliliters"),
    ("ml", "milliliter", "milliliters"),
    ("L", "liter", "liters"),
    ("KB", "kilobyte", "kilobytes"),
    ("kB", "kilobyte", "kilobytes"),
    ("MB", "megabyte", "megabytes"),
    ("GB", "gigabyte", "gigabytes"),
    ("TB", "terabyte", "terabytes"),
    ("Mbps", "megabit per second", "megabits per second"),
    ("Gbps", "gigabit per second", "gigabits per second"),
    ("mph", "mile per hour", "miles per hour"),
    ("kph", "kilometer per hour", "kilometers per hour"),
    ("AU", "astronomical unit", "astronomical units"),
    ("ly", "light-year", "light-years"),
];

/// Every number in `text` as words: "23,500" -> "twenty-three thousand five hundred",
/// "0.5875" and "0 point 5875" -> "zero point five eight seven five", "5 km/s" -> "five
/// kilometers per second", "40%" -> "forty percent", "21st" -> "twenty-first".
pub fn speak_numbers(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(text.len() * 2);
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        let currency = match character {
            '$' => Some(("dollar", "dollars")),
            '€' => Some(("euro", "euros")),
            '£' => Some(("pound", "pounds")),
            _ => None,
        };
        if let Some((one, many)) =
            currency.filter(|_| chars.get(index + 1).is_some_and(char::is_ascii_digit))
        {
            if out.chars().last().is_some_and(char::is_alphabetic) {
                out.push(' ');
            }
            let end = speak_number_at(&chars, index + 1, &mut out);
            let amount = chars[index + 1..end].iter().collect::<String>();
            out.push(' ');
            out.push_str(if amount == "1" { one } else { many });
            index = end;
            continue;
        }
        let starts_number = character.is_ascii_digit()
            || (matches!(character, '-' | '−')
                && chars.get(index + 1).is_some_and(char::is_ascii_digit)
                && index
                    .checked_sub(1)
                    .and_then(|previous| chars.get(previous))
                    .is_none_or(|previous| previous.is_whitespace() || "([=".contains(*previous)));
        if !starts_number {
            out.push(character);
            index += 1;
            continue;
        }
        if character != '-' && character != '−' {
            if let Some(previous) = out.chars().last() {
                if previous.is_alphabetic() {
                    out.push(' ');
                }
            }
        } else {
            out.push_str("minus ");
            index += 1;
        }
        index = speak_number_at(&chars, index, &mut out);
    }
    out
}

/// Speak the number starting at `start` and whatever qualifies it (decimals, percent, unit,
/// ordinal, magnitude). Returns the index after everything consumed.
fn speak_number_at(chars: &[char], start: usize, out: &mut String) -> usize {
    let (integer, grouped, mut index) = integer_run(chars, start);
    // A leading zero ("08") or a long unseparated run is an identifier, read digit by digit.
    let identifier = !grouped
        && ((integer.len() > 1 && integer.starts_with('0'))
            || integer.len() > LONGEST_SPOKEN_QUANTITY);
    let mut words = if identifier {
        digit_words(&integer)
    } else {
        let value = integer.parse::<u64>().unwrap_or(0);
        if !grouped
            && integer.len() == 4
            && (1100..2000).contains(&value)
            && !quantity_follows(chars, index)
        {
            year_words(value)
        } else {
            integer_words(value)
        }
    };
    let mut singular = integer == "1";

    // Fractional digits are read one by one: "0.60" is "zero point six zero". The desktop's
    // captions already write "0 point 60", which reads the same way.
    let mut decimal = false;
    loop {
        if chars.get(index) == Some(&'.') && chars.get(index + 1).is_some_and(char::is_ascii_digit)
        {
            let digits = digit_run(chars, index + 1);
            words.push_str(" point ");
            words.push_str(&digit_words(&digits));
            index += 1 + digits.len();
            decimal = true;
            singular = false;
            continue;
        }
        if let Some(after) = spoken_point(chars, index) {
            let digits = digit_run(chars, after);
            words.push_str(" point ");
            words.push_str(&digit_words(&digits));
            index = after + digits.len();
            decimal = true;
            singular = false;
            continue;
        }
        break;
    }
    out.push_str(&words);

    // A qualifier written directly after the digits.
    let letters = letter_run(chars, index);
    if !letters.is_empty() {
        if !decimal && ["st", "nd", "rd", "th"].contains(&letters.as_str()) {
            let ordinal = ordinal_words(&words);
            out.truncate(out.len() - words.len());
            out.push_str(&ordinal);
            return index + letters.len();
        }
        if let Some(name) = unit_name(&letters, singular) {
            out.push(' ');
            out.push_str(name);
            return speak_per_unit(chars, index + letters.len(), out);
        }
        if let Some(scale) = magnitude(&letters) {
            out.push(' ');
            out.push_str(scale);
            return index + letters.len();
        }
        out.push(' ');
        return index;
    }

    // A qualifier after one space: "40 %", "5 km", "30 °C".
    let spaced = index + usize::from(chars.get(index) == Some(&' '));
    match chars.get(spaced) {
        Some('%') => {
            out.push_str(" percent");
            return spaced + 1;
        }
        Some('°') => {
            out.push_str(if singular { " degree" } else { " degrees" });
            let scale = match chars.get(spaced + 1) {
                Some('C') if !chars.get(spaced + 2).is_some_and(|c| c.is_alphabetic()) => {
                    Some(" Celsius")
                }
                Some('F') if !chars.get(spaced + 2).is_some_and(|c| c.is_alphabetic()) => {
                    Some(" Fahrenheit")
                }
                _ => None,
            };
            if let Some(scale) = scale {
                out.push_str(scale);
                return spaced + 2;
            }
            return spaced + 1;
        }
        _ => {}
    }
    if spaced > index {
        let letters = letter_run(chars, spaced);
        if let Some(name) = unit_name(&letters, singular) {
            out.push(' ');
            out.push_str(name);
            return speak_per_unit(chars, spaced + letters.len(), out);
        }
    }
    index
}

/// "/s" or " per s" after a unit: "kilometers per second".
fn speak_per_unit(chars: &[char], index: usize, out: &mut String) -> usize {
    let after = if chars.get(index) == Some(&'/') {
        Some(index + 1)
    } else {
        let per = [' ', 'p', 'e', 'r', ' '];
        chars
            .get(index..index + per.len())
            .filter(|window| *window == per)
            .map(|_| index + per.len())
    };
    let Some(after) = after else {
        return index;
    };
    let letters = letter_run(chars, after);
    match unit_name(&letters, true) {
        Some(name) => {
            out.push_str(" per ");
            out.push_str(name);
            after + letters.len()
        }
        None => index,
    }
}

/// The digits of an integer, and whether they were written in comma groups ("23,500").
fn integer_run(chars: &[char], start: usize) -> (String, bool, usize) {
    let mut digits = digit_run(chars, start);
    let mut index = start + digits.len();
    let mut grouped = false;
    if digits.len() <= 3 {
        while chars.get(index) == Some(&',') {
            let group = digit_run(chars, index + 1);
            if group.len() != 3 {
                break;
            }
            digits.push_str(&group);
            index += 4;
            grouped = true;
        }
    }
    (digits, grouped, index)
}

fn digit_run(chars: &[char], start: usize) -> String {
    chars[start.min(chars.len())..]
        .iter()
        .take_while(|character| character.is_ascii_digit())
        .collect()
}

fn letter_run(chars: &[char], start: usize) -> String {
    chars[start.min(chars.len())..]
        .iter()
        .take_while(|character| character.is_alphabetic())
        .collect()
}

/// The index of the digits after " point " when the text already spells a decimal point.
fn spoken_point(chars: &[char], index: usize) -> Option<usize> {
    let point = [' ', 'p', 'o', 'i', 'n', 't', ' '];
    chars
        .get(index..index + point.len())
        .filter(|window| *window == point)
        .map(|_| index + point.len())
        .filter(|after| chars.get(*after).is_some_and(char::is_ascii_digit))
}

/// Whether a percent sign, degree sign, or unit follows, which makes a four-digit number a
/// quantity rather than a year.
fn quantity_follows(chars: &[char], index: usize) -> bool {
    let spaced = index + usize::from(chars.get(index) == Some(&' '));
    matches!(chars.get(spaced), Some('%' | '°'))
        || unit_name(&letter_run(chars, spaced), false).is_some()
}

fn unit_name(letters: &str, singular: bool) -> Option<&'static str> {
    UNITS
        .iter()
        .find(|(abbreviation, _, _)| *abbreviation == letters)
        .map(|(_, one, many)| if singular { *one } else { *many })
}

fn magnitude(letters: &str) -> Option<&'static str> {
    match letters {
        "K" => Some("thousand"),
        "M" => Some("million"),
        "B" | "bn" => Some("billion"),
        "T" => Some("trillion"),
        _ => None,
    }
}

fn digit_words(digits: &str) -> String {
    digits
        .chars()
        .filter_map(|digit| digit.to_digit(10))
        .map(|digit| ONES[digit as usize])
        .collect::<Vec<_>>()
        .join(" ")
}

/// "twenty-three thousand five hundred". Quantities too large to name are read digit by digit.
pub fn integer_words(value: u64) -> String {
    if value == 0 {
        return ONES[0].into();
    }
    let mut groups = Vec::new();
    let mut remaining = value;
    while remaining > 0 {
        groups.push((remaining % 1000) as usize);
        remaining /= 1000;
    }
    if groups.len() > SCALES.len() {
        return digit_words(&value.to_string());
    }
    let mut parts = Vec::new();
    for (scale, group) in groups.iter().enumerate().rev() {
        if *group == 0 {
            continue;
        }
        parts.push(below_thousand(*group));
        if !SCALES[scale].is_empty() {
            parts.push(SCALES[scale].into());
        }
    }
    parts.join(" ")
}

fn below_thousand(value: usize) -> String {
    let hundreds = value / 100;
    let rest = value % 100;
    let mut parts = Vec::new();
    if hundreds > 0 {
        parts.push(format!("{} hundred", ONES[hundreds]));
    }
    if rest > 0 {
        parts.push(below_hundred(rest));
    }
    parts.join(" ")
}

fn below_hundred(value: usize) -> String {
    if value < 20 {
        ONES[value].into()
    } else if value.is_multiple_of(10) {
        TENS[value / 10].into()
    } else {
        format!("{}-{}", TENS[value / 10], ONES[value % 10])
    }
}

/// Years before 2000 are said in pairs: "nineteen thirty", "nineteen oh five", "eleven hundred".
fn year_words(value: u64) -> String {
    let century = (value / 100) as usize;
    let rest = (value % 100) as usize;
    match rest {
        0 => format!("{} hundred", below_hundred(century)),
        1..=9 => format!("{} oh {}", below_hundred(century), ONES[rest]),
        _ => format!("{} {}", below_hundred(century), below_hundred(rest)),
    }
}

/// "twenty-one" -> "twenty-first".
fn ordinal_words(words: &str) -> String {
    let split = words.rfind([' ', '-']).map_or(0, |position| position + 1);
    let (head, last) = words.split_at(split);
    let ordinal = match last {
        "one" => "first".to_string(),
        "two" => "second".into(),
        "three" => "third".into(),
        "five" => "fifth".into(),
        "eight" => "eighth".into(),
        "nine" => "ninth".into(),
        "twelve" => "twelfth".into(),
        other if other.ends_with('y') => format!("{}ieth", &other[..other.len() - 1]),
        other => format!("{other}th"),
    };
    format!("{head}{ordinal}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_are_said_in_words() {
        assert_eq!(integer_words(0), "zero");
        assert_eq!(integer_words(23_500), "twenty-three thousand five hundred");
        assert_eq!(integer_words(3_000_000), "three million");
        assert_eq!(integer_words(1_000_001), "one million one");
        assert_eq!(
            speak_numbers("E equals 23,500"),
            "E equals twenty-three thousand five hundred"
        );
        assert_eq!(
            speak_numbers("10,000–8,000 BCE"),
            "ten thousand–eight thousand BCE"
        );
        assert_eq!(
            speak_numbers("In 2041 and 1930"),
            "In two thousand forty-one and nineteen thirty"
        );
        assert_eq!(
            speak_numbers("1905 and 1900"),
            "nineteen oh five and nineteen hundred"
        );
        assert_eq!(
            speak_numbers("1500 kg"),
            "one thousand five hundred kilograms"
        );
    }

    #[test]
    fn decimals_read_each_fractional_digit_whichever_way_they_arrive() {
        assert_eq!(speak_numbers("0.5875"), "zero point five eight seven five");
        assert_eq!(
            speak_numbers("0 point 5875"),
            "zero point five eight seven five"
        );
        assert_eq!(
            speak_numbers("98 point 5 percent"),
            "ninety-eight point five percent"
        );
        assert_eq!(speak_numbers("v1.2.3"), "v one point two point three");
        assert_eq!(
            speak_numbers("To the point 5 times."),
            "To the point five times."
        );
    }

    #[test]
    fn units_percentages_and_signs_are_named() {
        assert_eq!(
            speak_numbers("one 10,000 kg projectile at 5 km/s"),
            "one ten thousand kilograms projectile at five kilometers per second"
        );
        assert_eq!(speak_numbers("5 km per s"), "five kilometers per second");
        assert_eq!(speak_numbers("1 km"), "one kilometer");
        assert_eq!(
            speak_numbers("400ms and 99%"),
            "four hundred milliseconds and ninety-nine percent"
        );
        assert_eq!(
            speak_numbers("-40% at -12 °C"),
            "minus forty percent at minus twelve degrees Celsius"
        );
        assert_eq!(
            speak_numbers("3 point 75 times 10 to the power of 12 J"),
            "three point seven five times ten to the power of twelve joules"
        );
        assert_eq!(
            speak_numbers("$50M and $1"),
            "fifty million dollars and one dollar"
        );
        assert_eq!(speak_numbers("€2.50"), "two point five zero euros");
        assert_eq!(
            speak_numbers("5 in a row, 2 A-list"),
            "five in a row, two A-list"
        );
    }

    #[test]
    fn identifiers_ordinals_and_mixed_tokens_stay_readable() {
        assert_eq!(
            speak_numbers("the 21st and 3rd"),
            "the twenty-first and third"
        );
        assert_eq!(
            speak_numbers("the 12th and 40th"),
            "the twelfth and fortieth"
        );
        assert_eq!(speak_numbers("H2 and 3D"), "H two and three D");
        assert_eq!(
            speak_numbers("1930 08 05"),
            "nineteen thirty zero eight zero five"
        );
        assert_eq!(
            speak_numbers("serial 123456789"),
            "serial one two three four five six seven eight nine"
        );
        assert_eq!(speak_numbers("x-1 and 1-2"), "x-one and one-two");
    }
}
