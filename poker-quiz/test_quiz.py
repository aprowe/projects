import pytest
from quiz import QUESTIONS


def test_questions_exist():
    assert len(QUESTIONS) > 0


def test_each_question_has_required_fields():
    for q in QUESTIONS:
        assert "question" in q
        assert "choices" in q
        assert "answer" in q
        assert "explanation" in q


def test_answers_are_valid():
    for q in QUESTIONS:
        assert q["answer"] in ("A", "B", "C", "D")


def test_choices_count():
    for q in QUESTIONS:
        assert len(q["choices"]) == 4


def test_answer_choice_exists_in_choices():
    for q in QUESTIONS:
        letter = q["answer"]
        assert any(c.startswith(f"{letter})") for c in q["choices"])
