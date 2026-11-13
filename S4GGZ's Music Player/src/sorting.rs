#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortColumn {
    #[default]
    Library,
    Title,
    Artist,
    Duration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrackSort {
    pub column: SortColumn,
    pub descending: bool,
}

impl TrackSort {
    pub fn toggle(&mut self, column: SortColumn) {
        self.descending = if self.column == column {
            !self.descending
        } else {
            column == SortColumn::Duration
        };
        self.column = column;
    }

    pub fn label(self, column: SortColumn, title: &str) -> String {
        if self.column == column {
            format!("{title} {}", if self.descending { "↓" } else { "↑" })
        } else {
            title.to_owned()
        }
    }

    pub fn sql(self) -> &'static str {
        match (self.column, self.descending) {
            (SortColumn::Library, _) => "ar.name COLLATE NOCASE,al.title COLLATE NOCASE,COALESCE(t.disc_number,0),COALESCE(t.track_number,999),COALESCE(tt.title,t.title) COLLATE NOCASE,t.id",
            (SortColumn::Title, false) => "COALESCE(tt.title_key,t.title_key) ASC,ar.name_key,t.id",
            (SortColumn::Title, true) => "COALESCE(tt.title_key,t.title_key) DESC,ar.name_key,t.id",
            (SortColumn::Artist, false) => "ar.name_key ASC,COALESCE(tt.title_key,t.title_key),t.id",
            (SortColumn::Artist, true) => "ar.name_key DESC,COALESCE(tt.title_key,t.title_key),t.id",
            (SortColumn::Duration, false) => "t.duration_ms ASC,COALESCE(tt.title_key,t.title_key),t.id",
            (SortColumn::Duration, true) => "t.duration_ms DESC,COALESCE(tt.title_key,t.title_key),t.id",
        }
    }
}

/// Case-insensitive text ordering, including Romanian diacritics.
pub fn text_key(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|character| !('\u{0300}'..='\u{036f}').contains(character))
        .map(|character| match character {
            'ă' | 'â' => 'a',
            'î' => 'i',
            'ș' | 'ş' => 's',
            'ț' | 'ţ' => 't',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_clicks_toggle_and_duration_starts_with_longest() {
        let mut sort = TrackSort::default();
        sort.toggle(SortColumn::Title);
        assert!(!sort.descending);
        sort.toggle(SortColumn::Title);
        assert!(sort.descending);
        sort.toggle(SortColumn::Artist);
        assert!(!sort.descending);
        sort.toggle(SortColumn::Duration);
        assert!(sort.descending);
        sort.toggle(SortColumn::Duration);
        assert!(!sort.descending);
        assert_eq!(text_key("ȘȚĂÂÎ"), "staai");
    }
}
