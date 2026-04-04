import random

QUESTIONS = [
    {
        "question": "What is the best starting hand in Texas Hold'em?",
        "choices": ["A) Ace-King suited", "B) Pocket Aces", "C) Pocket Kings", "D) Ace-Queen suited"],
        "answer": "B",
        "explanation": "Pocket Aces (AA) is the strongest starting hand in Texas Hold'em.",
    },
    {
        "question": "How many cards are dealt to each player in Texas Hold'em?",
        "choices": ["A) 1", "B) 3", "C) 2", "D) 5"],
        "answer": "C",
        "explanation": "Each player receives 2 hole cards face down.",
    },
    {
        "question": "What is a 'Royal Flush'?",
        "choices": [
            "A) Any five cards of the same suit",
            "B) A, K, Q, J, 10 of the same suit",
            "C) Five consecutive cards of any suit",
            "D) Four of a kind plus an Ace",
        ],
        "answer": "B",
        "explanation": "A Royal Flush is A, K, Q, J, 10 all of the same suit — the highest possible hand.",
    },
    {
        "question": "What does 'the flop' refer to in Texas Hold'em?",
        "choices": [
            "A) The first community card dealt",
            "B) The second round of betting",
            "C) The first three community cards dealt face up",
            "D) Folding your hand",
        ],
        "answer": "C",
        "explanation": "The flop is the dealing of the first three community cards face up.",
    },
    {
        "question": "Which hand beats which?",
        "choices": [
            "A) Two pair beats three of a kind",
            "B) Flush beats full house",
            "C) Straight beats flush",
            "D) Full house beats flush",
        ],
        "answer": "D",
        "explanation": "A full house (three of a kind + a pair) ranks higher than a flush.",
    },
    {
        "question": "What is 'going all-in'?",
        "choices": [
            "A) Betting exactly half your chips",
            "B) Wagering all your remaining chips",
            "C) Calling any bet",
            "D) Raising the maximum allowed",
        ],
        "answer": "B",
        "explanation": "'All-in' means wagering all of your remaining chips on a single hand.",
    },
    {
        "question": "What is the 'big blind' in poker?",
        "choices": [
            "A) The largest bet allowed",
            "B) A mandatory bet placed by the player two seats left of the dealer",
            "C) Betting without looking at your cards",
            "D) The player with the most chips",
        ],
        "answer": "B",
        "explanation": "The big blind is a forced bet made by the player two positions to the left of the dealer button.",
    },
    {
        "question": "How many community cards are dealt in total in Texas Hold'em?",
        "choices": ["A) 3", "B) 4", "C) 5", "D) 6"],
        "answer": "C",
        "explanation": "Five community cards are dealt: three on the flop, one on the turn, and one on the river.",
    },
    {
        "question": "What is a 'bluff' in poker?",
        "choices": [
            "A) Folding a strong hand",
            "B) Betting or raising with a weak hand to make opponents fold",
            "C) Checking on every street",
            "D) Calling with the nuts",
        ],
        "answer": "B",
        "explanation": "A bluff is betting or raising with a weak hand hoping opponents will fold their better hands.",
    },
    {
        "question": "What hand rank is directly below a Full House?",
        "choices": ["A) Straight", "B) Three of a Kind", "C) Flush", "D) Two Pair"],
        "answer": "C",
        "explanation": "The hand rankings from high to low around that area: Full House > Flush > Straight.",
    },
]


def run_quiz(shuffle: bool = True) -> None:
    questions = QUESTIONS[:]
    if shuffle:
        random.shuffle(questions)

    score = 0
    total = len(questions)

    print("=== Poker Quiz ===\n")

    for i, q in enumerate(questions, 1):
        print(f"Question {i}/{total}: {q['question']}")
        for choice in q["choices"]:
            print(f"  {choice}")

        while True:
            answer = input("\nYour answer (A/B/C/D): ").strip().upper()
            if answer in ("A", "B", "C", "D"):
                break
            print("Please enter A, B, C, or D.")

        if answer == q["answer"]:
            print("Correct!\n")
            score += 1
        else:
            print(f"Wrong. The correct answer is {q['answer']}.")
            print(f"Explanation: {q['explanation']}\n")

    print(f"=== Quiz Complete ===")
    print(f"Score: {score}/{total} ({score / total * 100:.0f}%)")
    if score == total:
        print("Perfect score! You're a poker pro!")
    elif score >= total * 0.7:
        print("Nice work! You know your poker.")
    else:
        print("Keep studying — you'll get there!")


if __name__ == "__main__":
    run_quiz()
