import 'pretendard/dist/web/variable/pretendardvariable.css';
import './app.css';
import { mount } from 'svelte';
import App from './App.svelte';
import StudiumMini from './lib/StudiumMini.svelte';

const view = new URLSearchParams(window.location.search).get('studium-mini') === '1' ? StudiumMini : App;
mount(view, { target: document.getElementById('app')! });
