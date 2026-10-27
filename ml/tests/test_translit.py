from crisisgraph_ml.translit import skeleton
from crisisgraph_ml.triage_sft_v4_new import separation_problems


def test_malayalam_and_manglish_share_a_skeleton():
    assert skeleton("Ravile shwasam muttal") == skeleton("രാവിലെ ശ്വാസം മുട്ടൽ")
    assert skeleton("Vellam koodiyaal") == skeleton("vellam koodiyal")


def test_respelled_test_message_is_caught():
    # Frozen v3.1.0 case 095 with one word swapped and a respelling (batch 5 leak).
    record = {
        "id": "leak",
        "input": "Vellam koodiyal ammakku shwasam muttal varum, asthma und. Ippo "
        "kuzhappamilla. Njangal 2 per Kalarikkal lane il.",
        "target": {"victim_location": "Kalarikkal lane"},
    }
    assert any("transliterated" in problem for problem in separation_problems([record]))
