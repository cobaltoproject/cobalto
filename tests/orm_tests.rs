#[tokio::test]
async fn test_db_basic_crud() {
    use cobalto::orm::Db;
    use sqlx::FromRow;

    // 1. Create a minimal struct that matches the DB row
    #[derive(Debug, FromRow, PartialEq, Eq)]
    struct Person {
        name: String,
    }

    // 2. Connect and setup schema
    let db = Db::connect(":memory:").await.unwrap();
    db.execute("CREATE TABLE person (id INTEGER PRIMARY KEY, name TEXT)")
        .await
        .unwrap();
    db.execute("INSERT INTO person (name) VALUES ('Alice')")
        .await
        .unwrap();

    // 3. Fetch rows (using sqlx::FromRow)
    let people: Vec<Person> = db.fetch_all("SELECT name FROM person").await.unwrap();

    // 4. Extract names and assert
    let names: Vec<String> = people.into_iter().map(|person| person.name).collect();
    assert_eq!(names, vec!["Alice"]);
}
