//! A small condition-number grammar, never SQL or executable user text.
//! AND binds more tightly than OR; parentheses may nest eight levels. Every
//! existing condition appears exactly once, so the eight-condition cap remains.
use anyhow::{Result, ensure};

const MAX_BYTES: usize = 256;
const MAX_DEPTH: usize = 8;
const MAX_NODES: usize = 15;
#[derive(Clone, Debug)]
enum Node {
    Condition(usize),
    And(usize, usize),
    Or(usize, usize),
}
#[derive(Clone, Debug)]
pub struct Formula {
    nodes: Vec<Node>,
    root: usize,
    conditions: usize,
}
struct Parser<'a> {
    text: &'a [u8],
    at: usize,
    nodes: Vec<Node>,
    conditions: usize,
    used: u16,
}
impl Parser<'_> {
    fn spaces(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn token(&mut self, token: &[u8]) -> bool {
        self.spaces();
        let end = self.at + token.len();
        if self
            .text
            .get(self.at..end)
            .is_some_and(|v| v.eq_ignore_ascii_case(token))
            && (!token[0].is_ascii_alphabetic()
                || !self.text.get(end).is_some_and(u8::is_ascii_alphanumeric))
        {
            self.at = end;
            true
        } else {
            false
        }
    }
    fn push(&mut self, node: Node) -> Result<usize> {
        ensure!(
            self.nodes.len() < MAX_NODES,
            "A formula may use at most eight conditions"
        );
        self.nodes.push(node);
        Ok(self.nodes.len() - 1)
    }
    fn or(&mut self, depth: usize) -> Result<usize> {
        let mut left = self.and(depth)?;
        while self.token(b"OR") || self.token(b"|") {
            let right = self.and(depth)?;
            left = self.push(Node::Or(left, right))?;
        }
        Ok(left)
    }
    fn and(&mut self, depth: usize) -> Result<usize> {
        let mut left = self.condition(depth)?;
        while self.token(b"AND") || self.token(b"&") {
            let right = self.condition(depth)?;
            left = self.push(Node::And(left, right))?;
        }
        Ok(left)
    }
    fn condition(&mut self, depth: usize) -> Result<usize> {
        if self.token(b"(") {
            ensure!(
                depth < MAX_DEPTH,
                "Formula parentheses may nest at most eight levels"
            );
            let node = self.or(depth + 1)?;
            ensure!(self.token(b")"), "Close each formula parenthesis");
            return Ok(node);
        }
        self.spaces();
        let start = self.at;
        while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        let value = match self.text.get(start..self.at) {
            Some([digit @ b'1'..=b'8']) => usize::from(*digit - b'1'),
            _ => anyhow::bail!("Use a condition number (1–8), AND, OR and parentheses"),
        };
        ensure!(
            value < self.conditions,
            "Formula refers to a condition that does not exist"
        );
        let mask = 1u16 << value;
        ensure!(
            self.used & mask == 0,
            "Each condition must appear exactly once in the formula"
        );
        self.used |= mask;
        self.push(Node::Condition(value))
    }
}
impl Formula {
    pub fn parse(text: &str, conditions: usize) -> Result<Self> {
        ensure!(
            (1..=8).contains(&conditions),
            "Add one to eight conditions before entering a formula"
        );
        ensure!(
            text.len() <= MAX_BYTES && text.is_ascii(),
            "A formula is limited to 256 ASCII bytes"
        );
        let mut parser = Parser {
            text: text.as_bytes(),
            at: 0,
            nodes: Vec::with_capacity(MAX_NODES),
            conditions,
            used: 0,
        };
        let root = parser.or(0)?;
        parser.spaces();
        ensure!(
            parser.at == parser.text.len(),
            "Separate condition numbers with AND or OR"
        );
        ensure!(
            parser.used == (1u16 << conditions) - 1,
            "Each condition must appear exactly once in the formula"
        );
        Ok(Self {
            nodes: parser.nodes,
            root,
            conditions,
        })
    }
    pub fn evaluate(&self, values: &[bool]) -> Result<bool> {
        ensure!(
            values.len() == self.conditions,
            "Formula condition count changed"
        );
        fn node(formula: &Formula, index: usize, values: &[bool]) -> bool {
            match formula.nodes[index] {
                Node::Condition(index) => values[index],
                Node::And(a, b) => node(formula, a, values) && node(formula, b, values),
                Node::Or(a, b) => node(formula, a, values) || node(formula, b, values),
            }
        }
        Ok(node(self, self.root, values))
    }
    /// Invoke the parameterized leaf builder in traversal order. Reordered
    /// condition numbers must not accidentally bind another leaf's SQL values.
    pub fn sql(&self, leaf: &mut impl FnMut(usize) -> Result<String>) -> Result<String> {
        fn node(
            formula: &Formula,
            index: usize,
            leaf: &mut impl FnMut(usize) -> Result<String>,
        ) -> Result<String> {
            Ok(match formula.nodes[index] {
                Node::Condition(index) => format!("({})", leaf(index)?),
                Node::And(a, b) => format!(
                    "({} AND {})",
                    node(formula, a, leaf)?,
                    node(formula, b, leaf)?
                ),
                Node::Or(a, b) => format!(
                    "({} OR {})",
                    node(formula, a, leaf)?,
                    node(formula, b, leaf)?
                ),
            })
        }
        node(self, self.root, leaf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_precedence_and_parameter_order_are_explicit() -> Result<()> {
        let nested = Formula::parse("3 AND (1 OR (2 & 4))", 4)?;
        for bits in 0..16 {
            let values = (0..4).map(|n| bits & (1 << n) != 0).collect::<Vec<_>>();
            assert_eq!(
                nested.evaluate(&values)?,
                values[2] && (values[0] || (values[1] && values[3]))
            );
        }
        let mut order = Vec::new();
        let sql = nested.sql(&mut |index| {
            order.push(index);
            Ok(format!("field{index}=?"))
        })?;
        assert_eq!(order, [2, 0, 1, 3]);
        assert_eq!(
            sql,
            "((field2=?) AND ((field0=?) OR ((field1=?) AND (field3=?))))"
        );
        let precedence = Formula::parse("1 OR 2 AND 3", 3)?;
        assert!(precedence.evaluate(&[true, false, false])?);
        assert!(!precedence.evaluate(&[false, true, false])?);
        Ok(())
    }
    #[test]
    fn invalid_and_oversized_expressions_refuse_without_sql_interpretation() {
        for text in [
            "",
            "1",
            "1 OR 1",
            "0 OR 2",
            "1 OR 3",
            "1; DROP TABLE events",
            "1 UNION 2",
            "1 && 2",
            "1 || 2",
            "(1 OR 2",
            "1 OR 2)",
            "NOT 1 OR 2",
            "1 AND",
            "1 2",
            "11 OR 2",
            "é",
        ] {
            assert!(Formula::parse(text, 2).is_err(), "{text}");
        }
        assert!(Formula::parse(&format!("{}1 OR 2{}", "(".repeat(9), ")".repeat(9)), 2).is_err());
        assert!(Formula::parse(&format!("{}1 OR 2", " ".repeat(257)), 2).is_err());
        assert!(Formula::parse("1", 0).is_err());
        assert!(Formula::parse("1", 9).is_err());
        let maximum = Formula::parse("((((((((1)))))))) & 2 & 3 & 4 & 5 & 6 & 7 & 8", 8).unwrap();
        assert_eq!(maximum.nodes.len(), 15);
        assert!(maximum.evaluate(&[true; 8]).unwrap());
        assert!(maximum.evaluate(&[true; 7]).is_err());
    }
}
