# Resurse Aura Player

Temele se aleg din Setări → Temă. Structura este aceeași în surse și în
`target\release\assets`, folderul încărcat cu prioritate de executabil.

```text
assets/
  Aqua/
    bg.png
    bubble.png
    fish/nemo.png, fish1.png, fish2.png, fish3.png
  Beer/
    beer-bg.png
    beer.png
    beer-hand.png
  Disco/
    discoball.png
  discoball.png
  aura-icon.ico
  aura-icon.png
```

În surse, `Beer/beer-bg.avif` păstrează originalul furnizat cu extensie PNG.
Varianta `beer-bg.png` este conversia pentru program. `Beer/beer.png` este
imaginea furnizată inițial ca `beer-icon.png`. Sprite-urile păstrează transparența.
Nu sunt necesare recompilări când înlocuiești imaginile; repornește aplicația.

Tema **Disco** folosește `Disco/discoball.png`, o copie a imaginii furnizate.
Originalul `discoball.png` rămâne în rădăcina resurselor și este folosit dacă
lipsește copia din folderul temei. Oglinzile sunt desenate pe o suprafață
sferică animată, folosind interiorul imaginii pentru textură, cu fațete
multicolore iluminate, reflexe albe și un halou cald inspirate din referință;
podeaua și razele
sunt desenate în cod și nu necesită imagini suplimentare. Fundalul implicit
Disco este generat de aplicație; opțional se poate adăuga `Disco/disco-bg.png`.
Podeaua este o grilă de pătrate egale, proiectată în perspectivă pe întreaga
lățime a ferestrei, inclusiv la marginea ei îndepărtată. Bila emite separat
raze pe axele X, Y și Z: 24/18/12 pe axă, adică 72/54/36 în total pentru
Party Mode / În mișcare / Statică. Direcțiile sunt grupate în benzi ordonate,
fără raze apropiate de poli care ar părea fixe în timpul rotației. Punctele de
impact acoperă panourile vizibile și se adaptează unghiului. Muchiile meniurilor
pot reflecta lumina; reflexia la marginea ferestrei are o probabilitate de 18%
implicit, reglabilă în Setări, și este limitată la una pentru fiecare rază.
Fotografia aleasă din Setări este păstrată separat în
`%LOCALAPPDATA%\AuraPlayer\Disco\background.png` și apare sub bilă și podea;
panourile interfeței rămân deasupra efectelor.

## Iconițe comune opționale

Adaugă în rădăcina `assets` (în afara folderelor temelor):

| Fișier | Utilizare |
| --- | --- |
| home.png | Acasă |
| songs.png | Toate piesele / Explorează biblioteca |
| albums.png | Albume |
| favorites.png | Favorite |
| add-music.png | Import de folder / Descarcă melodii |
| rescan.png | Rescanează |
| settings.png | Setări |
| queue.png | Coada |
| equalizer.png | Egalizator |

PNG transparent de 64 × 64 px este recomandat; se afișează la aproximativ
18 × 18 px. Sunt acceptate și WebP/JPG cu aceleași nume; PNG are prioritate.
Simbolurile integrate sunt folosite dacă fișierele lipsesc. Butoanele pentru
pește, bulă, halbă, mână cu halbă, bilă disco și podea disco, inclusiv variantele tăiate, au iconițe
vectoriale monocrome integrate și nu cer alte imagini.
