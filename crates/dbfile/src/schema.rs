//! The shape of a SQLite file, compared with the shape a build creates.
//!
//! [`Schema::read`] takes the objects of a file as SQLite stores them (`sqlite_schema`), with the keys of the tables
//! that creation seeds. [`Schema::difference`] compares that with another schema (one a build makes in memory) and
//! names the first thing that differs. SQLite keeps a `CREATE` statement's text as it was written, comments and all,
//! so two texts that differ only in comments, whitespace or keyword case are the same: [`canonical`] decides that.

use std::collections::BTreeMap;
use std::fmt;

use rusqlite::Connection;

/// The objects of a database and the seeded keys of some of its tables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schema {
    /// (type, name) of each object, sorted.
    objects: BTreeMap<(String, String), Object>,
    /// `table.column` of each seeded table, with its keys (quoted, sorted).
    seeds: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Object {
    table: String,
    sql: String,
}

/// The first way two schemas differ. The kinds are SQLite's (`table`, `index`, `trigger`, `view`), and `rows` for a
/// seeded table's keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Difference {
    /// The expected schema has it and the file doesn't.
    Missing { kind: String, name: String },
    /// The file has it and the expected schema doesn't.
    Unexpected { kind: String, name: String },
    /// Both have it, with a different definition (or different keys).
    Changed { kind: String, name: String },
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Difference::Missing { kind, name } => write!(f, "{kind} {name} is missing"),
            Difference::Unexpected { kind, name } => write!(f, "{kind} {name} is not in this build's schema"),
            Difference::Changed { kind, name } => write!(f, "{kind} {name} differs from this build's"),
        }
    }
}

impl Schema {
    /// The schema of `conn`'s main database: every object except SQLite's own (`sqlite_*`), and the keys of each
    /// `(table, column)` in `seeds` whose table exists. A file that is not a database is an error (SQLite's
    /// `NotADatabase` code), as is any other failure to read it.
    pub fn read(conn: &Connection, seeds: &[(&str, &str)]) -> rusqlite::Result<Schema> {
        let mut stmt = conn.prepare(
            "SELECT type, name, tbl_name, sql FROM main.sqlite_schema
             WHERE sql IS NOT NULL AND substr(name, 1, 7) <> 'sqlite_' ORDER BY type, name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(((r.get::<_, String>(0)?, r.get::<_, String>(1)?), Object { table: r.get(2)?, sql: r.get(3)? }))
        })?;
        let mut objects = BTreeMap::new();
        for row in rows {
            let (key, object) = row?;
            objects.insert(key, object);
        }
        let mut seed_keys = BTreeMap::new();
        for &(table, column) in seeds {
            // A file of an older layout may lack the column: its keys can't be read, and the objects' comparison
            // reports the difference instead of this read failing.
            let has_column: i64 =
                conn.query_row("SELECT count(*) FROM pragma_table_info(?1) WHERE name = ?2", [table, column], |r| {
                    r.get(0)
                })?;
            if !objects.contains_key(&("table".to_string(), table.to_string())) || has_column == 0 {
                continue;
            }
            // The names are the caller's constants (the seeds of `pg-catalog` and `pg-thumbcache`), not input.
            let mut keys = conn.prepare(&format!("SELECT quote({column}) FROM main.{table} ORDER BY {column}"))?;
            let values = keys.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            seed_keys.insert(format!("{table}.{column}"), values);
        }
        Ok(Schema { objects, seeds: seed_keys })
    }

    /// Whether the database has no objects at all (a new, empty file).
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// The first difference between this schema (the file's) and `expected`, in this order: the objects of `expected`
    /// that are missing or changed, the objects this schema has that `expected` doesn't, then the seeded keys. `None`
    /// when they are the same.
    pub fn difference(&self, expected: &Schema) -> Option<Difference> {
        for ((kind, name), want) in &expected.objects {
            match self.objects.get(&(kind.clone(), name.clone())) {
                None => return Some(Difference::Missing { kind: kind.clone(), name: name.clone() }),
                // The stored text first: only text that differs needs canonicalizing.
                Some(have) if have.sql != want.sql && canonical(&have.sql) != canonical(&want.sql) => {
                    return Some(Difference::Changed { kind: kind.clone(), name: name.clone() });
                }
                Some(_) => {}
            }
        }
        for (kind, name) in self.objects.keys() {
            if !expected.objects.contains_key(&(kind.clone(), name.clone())) {
                return Some(Difference::Unexpected { kind: kind.clone(), name: name.clone() });
            }
        }
        for (key, want) in &expected.seeds {
            match self.seeds.get(key) {
                None => return Some(Difference::Missing { kind: "rows".to_string(), name: key.clone() }),
                Some(have) if have != want => {
                    return Some(Difference::Changed { kind: "rows".to_string(), name: key.clone() });
                }
                Some(_) => {}
            }
        }
        None
    }

    /// Whether the schema has a table of this name.
    pub fn has_table(&self, name: &str) -> bool {
        self.objects.contains_key(&("table".to_string(), name.to_string()))
    }
}

/// `sql` with what does not change its meaning taken out: comments and whitespace are dropped, unquoted words are
/// lowercased (ASCII only, as SQLite folds them), and each token is joined to the next with one space. Quoted strings
/// and quoted identifiers (`'..'`, `".."`, `` `..` ``, `[..]`) are kept exactly as written.
pub fn canonical(sql: &str) -> String {
    let mut tokens: Vec<String> = Vec::new();
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {}
            '-' if chars.peek() == Some(&'-') => {
                chars.next();
                for d in chars.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for d in chars.by_ref() {
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
            }
            '\'' | '"' | '`' => tokens.push(quoted(&mut chars, c, c)),
            '[' => tokens.push(quoted(&mut chars, '[', ']')),
            c if is_word(c) => {
                let mut word = String::new();
                word.push(c.to_ascii_lowercase());
                while let Some(d) = chars.peek().copied().filter(|&d| is_word(d)) {
                    word.push(d.to_ascii_lowercase());
                    chars.next();
                }
                tokens.push(word);
            }
            other => tokens.push(other.to_string()),
        }
    }
    tokens.join(" ")
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// A quoted token from `open` to `close`, both kept. For the quotes that escape by doubling (`''`), a doubled `close`
/// inside is part of the token.
fn quoted(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, open: char, close: char) -> String {
    let mut text = String::from(open);
    while let Some(d) = chars.next() {
        text.push(d);
        if d == close {
            if open != close || chars.peek() != Some(&close) {
                break;
            }
            if let Some(e) = chars.next() {
                text.push(e);
            }
        }
    }
    text
}

/// The `application_id` header field of `conn`'s main database (not part of the schema).
pub fn application_id(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA main.application_id", [], |r| r.get(0))
}

/// The `user_version` header field of `conn`'s main database.
pub fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA main.user_version", [], |r| r.get(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db(sql: &str) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(sql).unwrap();
        conn
    }

    #[test]
    fn comments_whitespace_and_keyword_case_do_not_count() {
        let a = canonical("CREATE TABLE t(a INTEGER, -- the first\n  b TEXT /* the second */)");
        assert_eq!(a, canonical("create   table t(a integer,b text)"));
        assert_eq!(a, "create table t ( a integer , b text )");
    }

    #[test]
    fn strings_and_quoted_identifiers_keep_their_case_and_their_text() {
        assert_ne!(canonical("SELECT 'Abc'"), canonical("SELECT 'abc'"));
        assert_ne!(canonical(r#"SELECT "Abc" FROM t"#), canonical(r#"SELECT "abc" FROM t"#));
        assert_ne!(canonical("SELECT [Abc] FROM t"), canonical("SELECT [abc] FROM t"));
        assert_eq!(canonical("SELECT 'it''s'"), "select 'it''s'");
        // A `--` or `/*` inside a string is text, not a comment.
        assert_eq!(canonical("SELECT 'a--b/*c'"), "select 'a--b/*c'");
    }

    #[test]
    fn only_ascii_is_folded() {
        assert_eq!(canonical("SELECT ÉA"), "select Éa");
    }

    #[test]
    fn a_definition_edit_is_a_difference_and_a_comment_edit_is_not() {
        let expected = Schema::read(&db("CREATE TABLE t(a INTEGER, b TEXT);"), &[]).unwrap();
        let commented = Schema::read(&db("CREATE TABLE t(a INTEGER, -- note\n b TEXT);"), &[]).unwrap();
        assert_eq!(commented.difference(&expected), None);
        let typed = Schema::read(&db("CREATE TABLE t(a TEXT, b TEXT);"), &[]).unwrap();
        assert_eq!(typed.difference(&expected), Some(Difference::Changed { kind: "table".into(), name: "t".into() }));
    }

    #[test]
    fn missing_and_unexpected_objects_are_named_expected_side_first() {
        let expected = Schema::read(&db("CREATE TABLE a(x); CREATE TABLE b(y);"), &[]).unwrap();
        let file = Schema::read(&db("CREATE TABLE a(x); CREATE TABLE c(z);"), &[]).unwrap();
        assert_eq!(file.difference(&expected), Some(Difference::Missing { kind: "table".into(), name: "b".into() }));
        let extra = Schema::read(&db("CREATE TABLE a(x); CREATE TABLE d(w);"), &[]).unwrap();
        let one = Schema::read(&db("CREATE TABLE a(x);"), &[]).unwrap();
        assert_eq!(extra.difference(&one), Some(Difference::Unexpected { kind: "table".into(), name: "d".into() }));
    }

    #[test]
    fn sqlite_internals_are_not_objects() {
        let conn = db("CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT UNIQUE); CREATE TABLE s(x);
             INSERT INTO t(v) VALUES ('a'); ANALYZE;");
        let schema = Schema::read(&conn, &[]).unwrap();
        assert!(schema.objects.keys().all(|(_, name)| !name.starts_with("sqlite_")), "{schema:?}");
        assert_eq!(schema.objects.len(), 2);
    }

    #[test]
    fn a_seeded_table_compares_its_keys_not_its_values() {
        let seeds = [("meta", "key")];
        let expected = Schema::read(
            &db("CREATE TABLE meta(key TEXT PRIMARY KEY, v); INSERT INTO meta VALUES ('a', 1), ('b', 2);"),
            &seeds,
        )
        .unwrap();
        let values = Schema::read(
            &db("CREATE TABLE meta(key TEXT PRIMARY KEY, v); INSERT INTO meta VALUES ('a', 9), ('b', 9);"),
            &seeds,
        )
        .unwrap();
        assert_eq!(values.difference(&expected), None);
        let fewer =
            Schema::read(&db("CREATE TABLE meta(key TEXT PRIMARY KEY, v); INSERT INTO meta VALUES ('a', 1);"), &seeds)
                .unwrap();
        assert_eq!(
            fewer.difference(&expected),
            Some(Difference::Changed { kind: "rows".into(), name: "meta.key".into() })
        );
        let none = Schema::read(&db("CREATE TABLE meta(key TEXT PRIMARY KEY, v);"), &seeds).unwrap();
        assert!(none.difference(&expected).is_some());
    }

    #[test]
    fn a_seed_column_the_file_lacks_is_a_difference_not_a_read_error() {
        let seeds = [("meta", "key")];
        let older = Schema::read(&db("CREATE TABLE meta(name TEXT PRIMARY KEY, v);"), &seeds).unwrap();
        let expected = Schema::read(&db("CREATE TABLE meta(key TEXT PRIMARY KEY, v);"), &seeds).unwrap();
        assert!(older.difference(&expected).is_some());
    }

    #[test]
    fn a_file_that_is_not_a_database_is_an_error_to_read() {
        let dir = pg_testkit::run_folder("scratch_dbfile", "pg-dbfile").unwrap();
        let path = dir.join("not-a-db.sqlite");
        std::fs::write(&path, vec![b'x'; 8192]).unwrap();
        let conn = Connection::open(&path).unwrap();
        let err = Schema::read(&conn, &[]).unwrap_err();
        assert_eq!(err.sqlite_error_code(), Some(rusqlite::ErrorCode::NotADatabase));
        drop(conn);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
