//! Symbol aliasing and lookup for security names/tickers across brokers.
use std::collections::HashMap;

/// Represents a mapping from alias (e.g., CUSIP or alt symbol) to canonical security symbol and note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolAlias {
    pub canonical: &'static str,
    pub aka: Option<&'static str>,
}

/// Provides lookup for symbol aliases.
pub struct SymbolAliasResolver {
    aliases: HashMap<&'static str, SymbolAlias>,
}

impl SymbolAliasResolver {
    pub fn new() -> Self {
        // Extend this mapping as needed for all brokers
        // The aliases can be CUSIPs, ISINs, or any other broker-specific identifiers that need
        // to be resolved to a canonical symbol.
        let aliases = HashMap::from([
            (
                "H038778",
                SymbolAlias {
                    canonical: "DLR.TO",
                    aka: Some("DLR.U.TO"),
                },
            ),
            (
                "G036247",
                SymbolAlias {
                    canonical: "DLR.TO",
                    aka: Some("DLR.U.TO"),
                },
            ),
            (
                "V009796",
                SymbolAlias {
                    canonical: "VEE.TO",
                    aka: None,
                },
            ),
            (
                "B074340",
                SymbolAlias {
                    canonical: "BMT CAD HISA",
                    aka: Some("BMT104/BMT109"),
                },
            ),
            (
                "B074356",
                SymbolAlias {
                    canonical: "BMT CAD HISA",
                    aka: Some("BMT104/BMT109"),
                },
            ),
            (
                "B074348",
                SymbolAlias {
                    canonical: "BMT USD HISA",
                    aka: Some("BMT124/BMT129"),
                },
            ),
            (
                "B074364",
                SymbolAlias {
                    canonical: "BMT USD HISA",
                    aka: Some("BMT124/BMT129"),
                },
            )
            // Add more aliases here
        ]);
        Self { aliases }
    }

    /// Returns the canonical symbol and note for a given alias, or None if the alias is not recognized.
    pub fn resolve(&self, symbol: &str) -> Option<&SymbolAlias> {
        self.aliases.get(symbol)
    }
}

// MARK: tests

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_resolve_known_alias() {
        let resolver = SymbolAliasResolver::new();
        let alias = resolver.resolve("H038778");
        assert!(alias.is_some());
        let alias_dupe = resolver.resolve("G036247");
        assert!(alias_dupe.is_some());
        assert_eq!(alias, alias_dupe);

        let alias = alias.unwrap();
        assert_eq!(alias.canonical, "DLR.TO");
        assert_eq!(alias.aka, Some("DLR.U.TO"));
    }
    #[test]
    fn test_resolve_unknown_alias() {
        let resolver = SymbolAliasResolver::new();
        assert!(resolver.resolve("UNKNOWN").is_none());
    }
}
