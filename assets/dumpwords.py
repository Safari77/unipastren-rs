#!/usr/bin/env python3
import sys

# Sallitut sanaluokat
ALLOWED_POS = {
    "[Ln]",  # Substantiivit (nimisana)
    "[Ll]",  # Adjektiivit (laatusana)
    "[Lnl]",  # Substantiivi-adjektiivit (nimi-laatusana)
    "[Lt]",  # Verbit (teonsana)
    "[Ls]",  # Adverbit (seikkasana)
    "[Lr]",  # Pronominit (asemosana)
    "[Lsu]",  # Pre-/postpositiot (suhdesana)
    "[Lc]",  # Konjunktiot (sidesana)
    "[Lh]",  # Huudahdussanat
    "[Lee]",  # Erisnimet: etunimet
    "[Les]",  # Erisnimet: sukunimet
    "[Lep]",  # Erisnimet: paikannimet
    "[Lem]",  # Erisnimet: muut
}

# Kielletyt tagit
# HUOM: "[Ef]" EI saa olla tässä listassa, koska Voikko käyttää [Ef]-tagia
# kaikkien verbien finiittisissä myöntömuodoissa (esim. "tapahtuu", "tapahtui", "juoksen").
# Liitepartikkelien (-han/-hän, -pa/-pä, -s) [Ef]-siirtymät suodatetaan erikseen alla!
FORBIDDEN_TAGS = {
    "[Bc]",
    "[Bm]",  # Yhdyssanarajat
    "[Xj]",  # Johdokset (estää -lainen, -mainen, -uus, -sti, -minen ym. ketjutuksen!)
    "[Fkin]",
    "[Fkaan]",
    "[Fko]",  # Liitepartikkelit (-kin, -kaan, -ko/-kö)
    "[O1y]",
    "[O2y]",
    "[O1m]",  # Omistusliitteet (-ni, -si, -mme, -nne, -nsa)
    "[O2m]",
    "[O3]",
    "[ls]",  # Sukija/laajennettu sanasto tai erikoisjohdos
    # Jos haluat VAIN perussijamuodot ilman adjektiivien komparatiivia (-mpi) ja superlatiivia (-in),
    # poista kommentti seuraavalta riviltä:
    # "[Ck]", "[Cs]",
}

MAX_DEPTH = 65


def parse_flag(sym):
    """Jäsentää HFST/VVFST-lippudiakriitin, esim. @U.VAIN.IND@ -> ('U', 'VAIN', 'IND')."""
    if len(sym) > 4 and sym.startswith("@") and sym.endswith("@"):
        parts = sym[1:-1].split(".")
        if len(parts) == 3:
            return (parts[0], parts[1], parts[2])
        elif len(parts) == 2:
            return (parts[0], parts[1], None)
    return None


def check_flag(flag, env):
    """
    Tarkistaa ja päivittää lippudiakriittien tilan (env).
    Huomioi oikein myös negaatioliput (@N.OMINAISUUS.ARVO@ -> "!ARVO").
    """
    op, feat, val = flag
    cur = env.get(feat)

    if op == "P":
        new_env = env.copy()
        new_env[feat] = val
        return True, new_env
    elif op == "C":
        new_env = env.copy()
        new_env.pop(feat, None)
        return True, new_env
    elif op == "N":
        new_env = env.copy()
        new_env[feat] = "!" + val
        return True, new_env
    elif op == "U":
        if cur is None:
            new_env = env.copy()
            new_env[feat] = val
            return True, new_env
        if cur.startswith("!"):
            if cur[1:] != val:
                new_env = env.copy()
                new_env[feat] = val
                return True, new_env
            return False, env
        return (cur == val), env
    elif op == "R":
        if val is None:
            return (cur is not None and not cur.startswith("!")), env
        return (cur == val), env
    elif op == "D":
        if val is None:
            return (cur is None or cur.startswith("!")), env
        if cur is not None and cur.startswith("!"):
            return True, env
        return (cur != val), env
    return True, env


def main():
    att_file = "vvfst/all.att"
    sys.stderr.write(f"Luetaan {att_file}...\n")

    transitions = {}
    final_states = set()

    with open(att_file, "r", encoding="utf-8") as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) >= 4:
                src = int(parts[0])
                dst = int(parts[1])
                surf_sym = parts[2]
                ana_sym = parts[3]

                # 1. Tarkistetaan kielletyt tagit
                if surf_sym in FORBIDDEN_TAGS or ana_sym in FORBIDDEN_TAGS:
                    continue

                # 2. Suodatetaan [Ef]-liitepartikkelit (-han/-hän, -pa/-pä, -pas/-päs, -s, -pi),
                # mutta SALLITAAN verbien finiittiset myöntömuodot, jotka myös käyttävät [Ef]-tagia!
                # Verkossa liitepartikkelien [Ef]-kaaret ovat tiloissa src < 10000 (esim. 79->12, 80->12, 283->12)
                # sekä vanhahtavassa -pi-päätteessä (dst == 210829) ja imperatiivin -s-päätteessä (210346, 212314).
                if (surf_sym == "[Ef]" or ana_sym == "[Ef]") and (
                    src < 10000 or dst in (12, 210829, 210346, 212314)
                ):
                    continue

                pos_tag = None
                is_xp = False
                skip = False

                for sym in (surf_sym, ana_sym):
                    if sym.startswith("[L") and sym.endswith("]"):
                        if sym not in ALLOWED_POS:
                            skip = True
                        else:
                            pos_tag = sym
                    elif sym == "[Xp]":
                        is_xp = True

                if skip:
                    continue

                surf_flag = parse_flag(surf_sym)
                ana_flag = parse_flag(ana_sym)
                flag = surf_flag or ana_flag

                # Huom: tarkistetaan VAIN surf_flag (ei ana_flag), jotta analyysipuolen lippu
                # ei vahingossa nollaa samalla kaarella olevaa pintapuolen kirjainta!
                if (
                    surf_sym in ("@0@", "ε")
                    or surf_flag is not None
                    or (surf_sym.startswith("[") and surf_sym.endswith("]"))
                ):
                    out_char = ""
                else:
                    out_char = surf_sym

                # Käytetään set():iä identtisten kaarien karsimiseen
                transitions.setdefault(src, set()).add(
                    (dst, out_char, flag, pos_tag is not None, is_xp)
                )
            elif len(parts) in (1, 2) and parts[0]:
                final_states.add(int(parts[0]))

    sys.stderr.write(
        f"Verkko ladattu: {len(transitions)} tilaa, {len(final_states)} lopputilaa.\n"
    )

    # Pinon alkio: (tila, sana, lippujen_tila, syvyys, vierailtu, pos_count, xp_count)
    stack = [(0, "", {}, 0, {(0, 0)}, 0, 0)]
    count = 0

    while stack:
        state, word, env, depth, visited, pos_cnt, xp_cnt = stack.pop()

        if state in final_states and word:
            if ":" not in word and "-" not in word:
                sys.stdout.write(word + "\n")
                count += 1
                if count % 100000 == 0:
                    sys.stderr.write(f"Generoitu {count} muotoa...\n")

        if depth >= MAX_DEPTH:
            continue

        for dst, out_char, flag, has_pos, has_xp in transitions.get(state, ()):
            new_word = word + out_char
            state_key = (dst, len(new_word))
            if state_key in visited:
                continue

            new_pos_cnt = pos_cnt + (1 if has_pos else 0)
            new_xp_cnt = xp_cnt + (1 if has_xp else 0)

            # ESTÄ JOHDOKSET: Sallitaan vain 1 sanaluokkatagi [L...] ja 1 perusmuototagi [Xp] per sana!
            if new_pos_cnt > 1 or new_xp_cnt > 1:
                continue

            if flag:
                ok, new_env = check_flag(flag, env)
                if not ok:
                    continue
            else:
                new_env = env

            stack.append(
                (
                    dst,
                    new_word,
                    new_env,
                    depth + 1,
                    visited | {state_key},
                    new_pos_cnt,
                    new_xp_cnt,
                )
            )

    sys.stderr.write(f"Valmis! Yhteensä {count} taivutusmuotoa.\n")


if __name__ == "__main__":
    main()
