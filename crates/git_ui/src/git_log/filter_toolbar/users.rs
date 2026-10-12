use std::collections::BTreeSet;

use collections::HashSet;
use git::repository::GitUser;

pub(crate) fn short_presentation(name: &str, email: &str) -> String {
    if !name.is_empty() {
        return name.to_string();
    }
    name_from_email(email).unwrap_or(email).to_string()
}

fn name_from_email(email: &str) -> Option<&str> {
    match email.find('@') {
        Some(at) if at > 0 => email.get(..at),
        _ => None,
    }
}

fn is_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn is_name_separator(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(character, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

fn is_word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_word_character)
}

fn is_printable_ascii(text: &str) -> bool {
    text.chars()
        .all(|character| (' '..='~').contains(&character))
}

pub(crate) fn first_and_last_name(name: &str) -> Option<(&str, &str)> {
    name.char_indices().rev().find_map(|(index, separator)| {
        if !is_name_separator(separator) {
            return None;
        }
        let first = name.get(..index)?;
        let last = name.get(index + separator.len_utf8()..)?;
        (is_word(first) && is_word(last)).then_some((first, last))
    })
}

fn name_to_lower_case(name: &str) -> String {
    if is_printable_ascii(name) {
        name.to_lowercase()
    } else {
        name.to_string()
    }
}

pub(crate) fn name_in_standard_form(name: &str) -> String {
    match first_and_last_name(name) {
        Some((first, last)) => format!("{} {}", first.to_lowercase(), last.to_lowercase()),
        None => name_to_lower_case(name),
    }
}

fn capitalize_name(name: &str) -> String {
    if !is_printable_ascii(name) {
        return name.to_string();
    }
    let mut characters = name.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

pub(crate) fn display_name(user: &GitUser) -> String {
    let short = short_presentation(&user.name, &user.email);
    match first_and_last_name(&short) {
        Some((first, last)) => format!("{} {}", capitalize_name(first), capitalize_name(last)),
        None => short,
    }
}

pub(crate) fn display_names(users: &[GitUser]) -> Vec<String> {
    users
        .iter()
        .map(display_name)
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect()
}

fn matches_standard_name(user: &GitUser, standard_name: &str) -> bool {
    let by_name = !user.name.is_empty() && name_in_standard_form(&user.name) == standard_name;
    let by_email = name_from_email(&user.email)
        .is_some_and(|email_name| name_in_standard_form(email_name) == standard_name);
    by_name || by_email
}

pub(crate) fn resolve_user_name(name: &str, all_users: &[GitUser]) -> Vec<GitUser> {
    let standard_name = name_in_standard_form(name);
    let mut seen = HashSet::default();
    all_users
        .iter()
        .filter(|user| matches_standard_name(user, &standard_name))
        .filter(|user| seen.insert((*user).clone()))
        .cloned()
        .collect()
}

pub(crate) fn resolve_current_user(current: &GitUser, all_users: &[GitUser]) -> Vec<GitUser> {
    let mut users = resolve_user_name(&current.name, all_users);
    let Some(email_name) = name_from_email(&current.email) else {
        return users;
    };
    let emails: HashSet<String> = users.iter().map(|user| user.email.to_lowercase()).collect();
    for candidate in resolve_user_name(email_name, all_users) {
        if emails.contains(&candidate.email.to_lowercase()) && !users.contains(&candidate) {
            users.push(candidate);
        }
    }
    users
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str, email: &str) -> GitUser {
        GitUser {
            name: name.to_string().into(),
            email: email.to_string().into(),
        }
    }

    #[test]
    fn first_and_last_name_are_split_on_any_ascii_punctuation_or_space() {
        for separator in "\"#$%&()*+,-./:;=?@[]^_`{|}~ ".chars() {
            let name = format!("Ivan{separator}Ivanov");
            assert_eq!(
                first_and_last_name(&name),
                Some(("Ivan", "Ivanov")),
                "separator {separator:?}"
            );
        }
        assert_eq!(first_and_last_name("a_b_c"), Some(("a_b", "c")));
        assert_eq!(first_and_last_name("Abc\u{016B}def"), None);
        assert_eq!(first_and_last_name("Ab\u{00E4}c\u{00F6}d"), None);
        assert_eq!(first_and_last_name("single"), None);
    }

    #[test]
    fn user_popup_lists_capitalized_unique_names_sorted() {
        let users = [
            user("ivan ivanov", "ivan@example.com"),
            user("Ivan Ivanov", "ivan@work.com"),
            user("", "petr@example.com"),
            user("Zoe", "zoe@example.com"),
        ];
        assert_eq!(display_names(&users), vec!["Ivan Ivanov", "Zoe", "petr"]);
    }

    #[test]
    fn names_resolve_to_every_matching_identity_by_name_or_email() {
        let users = [
            user("Ivan Ivanov", "ivan@example.com"),
            user("ivan.ivanov", "other@example.com"),
            user("Someone", "ivan_ivanov@example.com"),
            user("Petr", "petr@example.com"),
        ];
        let resolved = resolve_user_name("Ivan Ivanov", &users);
        assert_eq!(resolved, users[..3].to_vec());
        assert!(resolve_user_name("nobody", &users).is_empty());
    }

    #[test]
    fn current_user_adds_email_matches_only_for_known_emails() {
        let users = [
            user("Ivan Ivanov", "ivan@ivanov.com"),
            user("Other", "ivan@petrov.com"),
            user("ivan", "IVAN@ivanov.com"),
        ];
        let current = user("Ivan Ivanov", "ivan@ivanov.com");
        assert_eq!(
            resolve_current_user(&current, &users),
            vec![users[0].clone(), users[2].clone()]
        );
    }
}
