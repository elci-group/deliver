struct Button;

impl Button {
    fn set_text(&self, text: &str) {
        let _ = text;
    }
}

fn main() {
    println!("Welcome to Ogma Demo");
    println!("Choose an option");
    let button = Button;
    button.set_text("Delete account");
    if std::env::args().count() == 0 {
        panic!("internal invariant");
    }
}
