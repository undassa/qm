//! Раскрыватель перечня имён: `FR-CFG-01…15` — это пятнадцать требований.
//!
//! Один на весь набор. Документы называют имена сокращённо — диапазоном,
//! списком, косой чертой, — и всякая проверка покрытия обязана читать их
//! ОДИНАКОВО. Два разных чтения дают две разные правды об одном и том же
//! перечне, и спорить будет не с чем: обе «посчитаны».
//!
//! Разбор написан руками, а не одним образцом: в исходном образце стояли
//! просмотры назад и вперёд (`(?<![A-Za-z0-9-])`, `(?!\d)`), которых `regex`
//! не умеет. Ручной проход границы проверяет прямо и читается построчно.

/// Приставки, которые считаются именами набора. `ADR` и `Q` сюда НЕ входят:
/// у них своя нумерация и свои документы, и раскрывать их диапазоном нечем.
const PREFIXES: [&str; 6] = ["NFR", "FR", "TC", "ST", "US", "SCR"];

#[derive(Clone, Copy, PartialEq)]
struct Num {
    n: u32,
    suf: Option<char>,
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

/// Число имени: одна–три цифры и необязательная строчная буква. За ним не
/// должно стоять цифры — иначе `2026` прочиталось бы как `202` и год стал бы
/// именем требования.
fn read_num(b: &[char], mut i: usize) -> Option<(Num, usize)> {
    let start = i;
    while i < b.len() && b[i].is_ascii_digit() && i - start < 3 {
        i += 1;
    }
    if i == start {
        return None;
    }
    let n: u32 = b[start..i].iter().collect::<String>().parse().ok()?;
    let mut suf = None;
    if i < b.len() && b[i].is_ascii_lowercase() {
        suf = Some(b[i]);
        i += 1;
    }
    if i < b.len() && b[i].is_ascii_digit() {
        return None;
    }
    Some((Num { n, suf }, i))
}

/// Разделитель диапазона. Дефис сюда НЕ входит намеренно: `FR-ESC-06-09` —
/// это одно имя со странным хвостом, а не четыре имени. Дефис уже занят
/// внутри самого имени, и второе значение сделало бы разбор гадательным.
fn read_dash(b: &[char], mut i: usize) -> Option<usize> {
    while i < b.len() && b[i] == ' ' {
        i += 1;
    }
    let mut j = i;
    if b.get(j) == Some(&'…') || b.get(j) == Some(&'–') || b.get(j) == Some(&'—') {
        j += 1;
    } else if b.len() >= j + 3 && b[j] == '.' && b[j + 1] == '.' && b[j + 2] == '.' {
        j += 3;
    } else {
        return None;
    }
    while j < b.len() && b[j] == ' ' {
        j += 1;
    }
    Some(j)
}

fn skip_tick(b: &[char], i: usize) -> usize {
    if b.get(i) == Some(&'`') { i + 1 } else { i }
}

/// Приставка на позиции: `FR-CFG` или просто `NFR`. Возвращает саму приставку
/// и место после неё.
fn read_prefix(b: &[char], i: usize) -> Option<(String, usize)> {
    let s: String = b[i..].iter().take(10).collect();
    let head = PREFIXES.iter().find(|p| s.starts_with(**p))?;
    let mut j = i + head.len();
    // Уточнение приставки: `-CFG`, две–шесть заглавных.
    if b.get(j) == Some(&'-') {
        let mut k = j + 1;
        let letters = {
            let mut n = 0;
            while k < b.len() && b[k].is_ascii_uppercase() && n < 6 {
                k += 1;
                n += 1;
            }
            n
        };
        if letters >= 2 && b.get(k) == Some(&'-') {
            j = k;
        }
    }
    if b.get(j) != Some(&'-') {
        return None;
    }
    Some((b[i..j].iter().collect(), j + 1))
}

/// Раскрывает текст в имена, по порядку первого появления.
pub fn expand(text: &str) -> Vec<String> {
    let b: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut i = 0usize;
    while i < b.len() {
        // Граница слева: имя не начинается посреди `nfr-01-oncall` или `x-FR-01`.
        let opened = b[i] == '`';
        let at = if opened { i + 1 } else { i };
        let left_ok = if opened {
            true
        } else {
            i == 0 || !is_id_char(b[i - 1])
        };
        let Some((pre, mut j)) = read_prefix(&b, at).filter(|_| left_ok) else {
            i += 1;
            continue;
        };
        let Some((head, after)) = read_num(&b, j) else {
            i += 1;
            continue;
        };
        j = skip_tick(&b, after);
        let name = |n: u32, suf: Option<char>| match suf {
            Some(c) => format!("{pre}-{n:02}{c}"),
            None => format!("{pre}-{n:02}"),
        };
        let mut take = |n: u32, suf: Option<char>, out: &mut Vec<String>| {
            let id = name(n, suf);
            if seen.insert(id.clone()) {
                out.push(id);
            }
        };
        take(head.n, head.suf, &mut out);
        let (mut last, mut last_suf) = (head.n, head.suf);

        // Хвост: сколько угодно продолжений подряд.
        loop {
            let mut k = j;
            // а) диапазон: `…09`, `…FR-GAP-12`
            if let Some(d) = read_dash(&b, k) {
                let mut m = skip_tick(&b, d);
                if let Some((p2, after_pre)) = read_prefix(&b, m) {
                    if p2 == pre {
                        m = after_pre;
                    }
                }
                if let Some((end, after)) = read_num(&b, m) {
                    // Буквенный диапазон работает ВНУТРИ одного числа:
                    // `05a…05d` — это a, b, c, d. Через число букву не тянем:
                    // `05a…06c` даёт `05a` и `06`, а не выдуманный ряд.
                    if last_suf.is_some() && end.suf.is_some() && end.n == last {
                        let from = last_suf.unwrap() as u8 + 1;
                        for c in from..=end.suf.unwrap() as u8 {
                            take(end.n, Some(c as char), &mut out);
                        }
                        last_suf = end.suf;
                    } else {
                        for n in (last + 1)..=end.n {
                            take(n, None, &mut out);
                        }
                        last = end.n;
                        last_suf = None;
                    }
                    j = skip_tick(&b, after);
                    continue;
                }
            }
            // б) косая черта: `/14`
            let mut m = k;
            while m < b.len() && b[m] == ' ' {
                m += 1;
            }
            if b.get(m) == Some(&'/') {
                m = skip_tick(&b, m + 1);
                if let Some((e, after)) = read_num(&b, m) {
                    take(e.n, e.suf, &mut out);
                    last = e.n;
                    last_suf = e.suf;
                    j = skip_tick(&b, after);
                    continue;
                }
            }
            // в) перечисление: `, 07`, ` · 02`, и внутри него свой диапазон
            k = j;
            while k < b.len() && b[k] == ' ' {
                k += 1;
            }
            if b.get(k) == Some(&',') || b.get(k) == Some(&'·') {
                let mut m = k + 1;
                while m < b.len() && b[m] == ' ' {
                    m += 1;
                }
                m = skip_tick(&b, m);
                if let Some((one, after)) = read_num(&b, m) {
                    take(one.n, one.suf, &mut out);
                    last = one.n;
                    last_suf = one.suf;
                    let mut p = skip_tick(&b, after);
                    if let Some(d) = read_dash(&b, p) {
                        let q = skip_tick(&b, d);
                        if let Some((e, a2)) = read_num(&b, q) {
                            for n in (last + 1)..=e.n {
                                take(n, None, &mut out);
                            }
                            last = e.n;
                            last_suf = None;
                            p = skip_tick(&b, a2);
                        }
                    }
                    j = p;
                    continue;
                }
            }
            break;
        }
        i = j.max(i + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::expand;

    /// Тридцать одна запись — не украшение: каждая закрывает случай, на
    /// котором прежний разбор ошибался. Правка раскрывателя, роняющая хоть
    /// одну, меняет смысл ВСЕХ проверок покрытия разом.
    const FIXTURES: [(&str, &str); 31] = [
        ("FR-ESC-06…09", "FR-ESC-06 FR-ESC-07 FR-ESC-08 FR-ESC-09"),
        ("`FR-GAP-07…FR-GAP-12`", "FR-GAP-07 FR-GAP-08 FR-GAP-09 FR-GAP-10 FR-GAP-11 FR-GAP-12"),
        ("FR-SIT-04, 07, 09", "FR-SIT-04 FR-SIT-07 FR-SIT-09"),
        ("FR-ESC-13/14", "FR-ESC-13 FR-ESC-14"),
        ("TC-SIT-17/19/20", "TC-SIT-17 TC-SIT-19 TC-SIT-20"),
        ("ST-33–41, 43, 44",
         "ST-33 ST-34 ST-35 ST-36 ST-37 ST-38 ST-39 ST-40 ST-41 ST-43 ST-44"),
        ("| NFR-05 ✓уд | Ни одна доставка не теряется |", "NFR-05"),
        ("| `NFR-22`, `NFR-24` | доступность приёма |", "NFR-22 NFR-24"),
        ("`SCR-SET-09` · `SCR-SHELL-07`", "SCR-SET-09 SCR-SHELL-07"),
        ("FR-STP-01 · 02 · 03 · 14", "FR-STP-01 FR-STP-02 FR-STP-03 FR-STP-14"),
        ("US-ST-01 · 02 · 09", "US-ST-01 US-ST-02 US-ST-09"),
        ("`TC-SIT-23a`", "TC-SIT-23a"),
        ("`TC-PAG-08d`/`08e`", "TC-PAG-08d TC-PAG-08e"),
        ("`TC-SIT-23a`, `24b`", "TC-SIT-23a TC-SIT-24b"),
        ("TC-ESC-01, 05, 05b, 05c, 05d", "TC-ESC-01 TC-ESC-05 TC-ESC-05b TC-ESC-05c TC-ESC-05d"),
        ("TC-SIT-01…03a", "TC-SIT-01 TC-SIT-02 TC-SIT-03"),
        ("TC-ESC-05a…05d", "TC-ESC-05a TC-ESC-05b TC-ESC-05c TC-ESC-05d"),
        ("`TC-ESC-05a…TC-ESC-05c`", "TC-ESC-05a TC-ESC-05b TC-ESC-05c"),
        ("TC-PAG-08a–08c", "TC-PAG-08a TC-PAG-08b TC-PAG-08c"),
        ("TC-ESC-05a…06c", "TC-ESC-05a TC-ESC-06"),
        ("FR-ESC-06-09", "FR-ESC-06"),
        ("`FR-STP-01` · `FR-STP-02`\n`FR-STP-03`", "FR-STP-01 FR-STP-02 FR-STP-03"),
        ("`TC-SIT-01`, `02`, `03`, `11`", "TC-SIT-01 TC-SIT-02 TC-SIT-03 TC-SIT-11"),
        ("`FR-SIT-02`, `03`, `05`, `06`, `08`, `10`",
         "FR-SIT-02 FR-SIT-03 FR-SIT-05 FR-SIT-06 FR-SIT-08 FR-SIT-10"),
        ("`US-ST-01`, 02, 03, 04, 09", "US-ST-01 US-ST-02 US-ST-03 US-ST-04 US-ST-09"),
        ("`FR-ORG-24` — `TC-ORG-28`", "FR-ORG-24 TC-ORG-28"),
        ("`TC-SIT-05` — `TC-SIT-08`", "TC-SIT-05 TC-SIT-06 TC-SIT-07 TC-SIT-08"),
        ("FR-SIT-01, 2026 год", "FR-SIT-01"),
        ("требование `FR-SIT-01`, 5 проверок", "FR-SIT-01 FR-SIT-05"),
        ("nfr-01-oncall.js", ""),
        ("ADR-0088 и Q-224", ""),
    ];

    #[test]
    fn fixtures_hold() {
        let mut bad = Vec::new();
        for (input, want) in FIXTURES {
            let got = expand(input).join(" ");
            if got != want {
                bad.push(format!("{input:?}\n  ждали: {want}\n  вышло: {got}"));
            }
        }
        assert!(bad.is_empty(), "разошлось {}:\n{}", bad.len(), bad.join("\n"));
    }
}

/// Имена, написанные ЦЕЛИКОМ, без раскрытия перечня.
///
/// Раскрыватель нужен там, где перечень объявляет покрытие: `FR-CFG-01…15` —
/// пятнадцать требований. Там его щедрость безвредна, лишнее имя лишь
/// расширяет множество покрытого. Для указателя она губительна: строка
/// «`NFR-02` — 272 FR и 32 NFR» прочиталась бы диапазоном до двухсот
/// семидесяти двух, и правило «имя ведёт в никуда» нашло бы двести
/// выдуманных имён. Указатель называет имя, а не перечень.
pub fn plain(text: &str) -> Vec<String> {
    let b: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut i = 0usize;
    while i < b.len() {
        let left_ok = i == 0 || !is_id_char(b[i - 1]);
        if !left_ok {
            i += 1;
            continue;
        }
        let Some((pre, j)) = read_prefix(&b, i) else { i += 1; continue };
        let Some((num, after)) = read_num(&b, j) else { i += 1; continue };
        // Справа — не буква и не цифра: `FR-SIT-01a` целиком, `FR-SIT-012` нет.
        if after < b.len() && is_id_char(b[after]) {
            i += 1;
            continue;
        }
        let id = match num.suf {
            Some(c) => format!("{pre}-{:02}{c}", num.n),
            None => format!("{pre}-{:02}", num.n),
        };
        if seen.insert(id.clone()) {
            out.push(id);
        }
        i = after.max(i + 1);
    }
    out
}

/// Голые числа, оставшиеся в СТРОКЕ-ПЕРЕЧНЕ после раскрытия.
///
/// Строка вида `` `FR-SIT-01`, 03, 07 `` — перечень: раскрыватель съедает
/// каждое имя целиком. Оставшееся число значит, что запись прячет имя формой,
/// которой раскрыватель не знает, — и перечень, читаемый глазом как полный,
/// на деле короче. Строка, где остались СЛОВА, перечнем не считается: это
/// проза, и числа в ней говорят не об именах.
pub fn hidden_numbers(text: &str) -> Vec<String> {
    let b: Vec<char> = text.chars().collect();
    // Границы того, что раскрыватель разобрал: повторяем его проход и
    // отмечаем съеденное.
    let mut eaten = vec![false; b.len()];
    let mut i = 0usize;
    while i < b.len() {
        let opened = b[i] == '`';
        let at = if opened { i + 1 } else { i };
        let left_ok = opened || i == 0 || !is_id_char(b[i - 1]);
        let Some((_, j)) = read_prefix(&b, at).filter(|_| left_ok) else {
            i += 1;
            continue;
        };
        let Some((_, after)) = read_num(&b, j) else {
            i += 1;
            continue;
        };
        // Съедено само имя и весь его хвост: диапазоны, косые, перечисление.
        let mut end = skip_tick(&b, after);
        loop {
            let mut moved = false;
            if let Some(d) = read_dash(&b, end) {
                let mut m = skip_tick(&b, d);
                if let Some((_, ap)) = read_prefix(&b, m) {
                    m = ap;
                }
                if let Some((_, a2)) = read_num(&b, m) {
                    end = skip_tick(&b, a2);
                    moved = true;
                }
            }
            let mut k = end;
            while k < b.len() && b[k] == ' ' {
                k += 1;
            }
            if !moved && (b.get(k) == Some(&'/') || b.get(k) == Some(&',') || b.get(k) == Some(&'·')) {
                let mut m = skip_tick(&b, k + 1);
                while m < b.len() && b[m] == ' ' {
                    m += 1;
                }
                m = skip_tick(&b, m);
                if let Some((_, a2)) = read_num(&b, m) {
                    end = skip_tick(&b, a2);
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
        for e in eaten.iter_mut().take(end).skip(i) {
            *e = true;
        }
        i = end.max(i + 1);
    }
    let rest: String = b
        .iter()
        .enumerate()
        .map(|(k, c)| if eaten[k] { ' ' } else { *c })
        .collect();
    // Образцы собираются ОДИН раз на всю жизнь процесса. Собирались они здесь,
    // внутри — а зовут эту работу на каждую строку каждого документа, и сборка
    // образца дороже самого поиска. Сорок одна секунда из сорока трёх, что
    // занимала пересборка набора, уходила ровно сюда.
    static WORDS: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\p{L}{2,}").expect("образец слова"));
    static BARE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?:^|[^\p{L}0-9-])([0-9]{1,3}[a-z]?)(?:$|[^\p{L}0-9])")
            .expect("образец голого числа")
    });
    // Остались слова — это проза, а не перечень.
    if WORDS.is_match(&rest) {
        return Vec::new();
    }
    BARE.captures_iter(&rest).map(|c| c[1].to_owned()).collect()
}

/// Имена ПЕРВОЙ ЯЧЕЙКИ строки таблицы — от первой палки до второй.
///
/// Имя первой ячейкой документ объявляет СВОИМ: так перечисляют состав. То же
/// имя в прозе («тот же принцип, что у `FR-SIG-10`») — ссылка на чужое. Правило
/// короткой формы считало их одним и находило одиннадцать нарушений там, где
/// исходник находил ноль: все одиннадцать были перекрёстными ссылками.
///
/// Строка НЕ таблицы даёт пусто, а не имена всей строки: иначе «первой ячейкой»
/// оказалось бы всякое имя в прозе, и различие исчезло бы.
pub fn heading_cell(line: &str) -> Vec<String> {
    match line.trim_start().strip_prefix('|') {
        Some(rest) => expand(rest.split('|').next().unwrap_or("")),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod ячейка {
    use super::heading_cell;

    #[test]
    fn первая_ячейка_отдаёт_своё_имя() {
        assert_eq!(heading_cell("| `FR-SIG-10` | держит подпись |"), vec!["FR-SIG-10"]);
    }

    #[test]
    fn проза_не_первая_ячейка() {
        assert!(heading_cell("тот же принцип, что у `FR-SIG-10`").is_empty());
    }

    #[test]
    fn имя_из_второй_ячейки_не_считается_своим() {
        assert!(heading_cell("| держит | `FR-SIG-10` |").is_empty());
    }

    #[test]
    fn перечень_в_первой_ячейке_раскрывается() {
        // `FR-SIT-04, 07, 09` — три имени состава, а не одно.
        let got = heading_cell("| `FR-SIT-04, 07, 09` | наблюдение |");
        assert_eq!(got.len(), 3, "раскрылось: {got:?}");
        assert!(got.contains(&"FR-SIT-09".to_owned()), "{got:?}");
    }

    #[test]
    fn разделитель_таблицы_имён_не_даёт() {
        assert!(heading_cell("| --- | --- |").is_empty());
    }
}
